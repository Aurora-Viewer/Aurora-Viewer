// Aurora LSL Bridge query and posing-consent services v1.40: the Legacy Viewer's getZOffsets
// and getScriptInfo equivalents (fsradar.cpp, fs_resources/EBEDD1D2-*.lsltxt,
// originally LGPL 2.1). A separate script beside the movement controller:
// Mono gives each script its own 64 KB, and a query can never halt Movelock.
// It has its own HTTP-in URL and token, and never touches physics.
string FOLDER = "__FOLDER__";
string VERSION = "1.40";
string gURL;
key gToken;
key gURLRequest;
key gOwner;
integer DISCOVERY_CHANNEL = -184735219;
integer gListener;
integer gPoseListener;
integer gPortalListener;
integer PORTAL_CHANNEL = -184735221;
list gPortalPeers;
integer POSE_CHANNEL = -184735220;
list gPosePeers;
integer gPaired;
float gNextHello;

// Keys have a validity test in conditions; LSL's ! operator requires an integer.
integer validKey(key value) {
    if (value) return TRUE;
    return FALSE;
}

announce() {
    if (gPaired && llGetAttached() && gURL != "") llOwnerSay("<auroraBridgeQuery>" + VERSION + "|" + FOLDER + "|" + gURL + "|" + (string)gToken + "</auroraBridgeQuery>");
}
requestURL() {
    llSetTimerEvent(0.0);
    llReleaseURL(gURL);
    gURL = "";
    gToken = llGenerateKey();
    gURLRequest = llRequestSecureURL();
}
initialize() {
    gPaired = FALSE;
    gNextHello = 0.0;
    gOwner = llGetOwner();
    llListenRemove(gListener);
    // Same owner-filtered, root-addressed probe as the movement controller.
    gListener = llListen(DISCOVERY_CHANNEL, "", gOwner, "auroraHello|" + (string)llGetKey());
    llListenRemove(gPoseListener);
    gPoseListener = llListen(POSE_CHANNEL, "", NULL_KEY, "");
    gPosePeers = [];
    llListenRemove(gPortalListener);
    gPortalListener = llListen(PORTAL_CHANNEL, "", NULL_KEY, "");
    gPortalPeers = [];
    requestURL();
}
query(key request, string kind, string targets) {
    if (kind == "portal") {
        // The viewer validates the typed, base64 invitation. The script only
        // authenticates its sender and relays bounded region-local data.
        key target = (key)llGetSubString(targets, 0, 35);
        string payload = llDeleteSubString(targets, 0, 36);
        if (llStringLength(targets) < 41 || llStringLength(payload) > 640
            || llGetSubString(targets, 36, 36) != ","
            || (!validKey(target) && llGetSubString(targets, 0, 35) != (string)NULL_KEY)) {
            llHTTPResponse(request, 400, "invalid"); return;
        }
        // Private detail requests/replies can never use the regional broadcast.
        if (!validKey(target)) {
            string decoded = llBase64ToString(payload);
            if (llJsonValueType(decoded, ["Details"]) != JSON_INVALID
                || llJsonValueType(decoded, ["Request"]) != JSON_INVALID) {
                llHTTPResponse(request, 400, "invalid"); return;
            }
        }
        if (target) llRegionSayTo(target, PORTAL_CHANNEL, "portal2|" + payload);
        else llRegionSay(PORTAL_CHANNEL, "portal2|" + payload);
        llHTTPResponse(request, 200, "[]");
    } else if (kind == "pose") {
        list fields = llParseStringKeepNulls(targets, [","], []);
        string action = llList2String(fields, 0);
        key nonce = llList2Key(fields, 1);
        key target = llList2Key(fields, 2);
        if (llGetListLength(fields) != 3 || !validKey(nonce) || !validKey(target) || target == gOwner
            || llListFindList(["request", "accept", "refuse", "cancel", "withdraw"], [action]) < 0) {
            llHTTPResponse(request, 400, "invalid"); return;
        }
        // Explicit commands only: no advertisement, discovery or viewer probe.
        llRegionSayTo(target, POSE_CHANNEL, "pose1|" + (string)nonce + "|" + action + "|" + (string)target);
        llHTTPResponse(request, 200, "[]");
    } else if (kind == "altitudes") {
        // Comma-separated 36-character keys, read at a fixed stride.
        integer length = llStringLength(targets);
        integer count = (length + 1) / 37;
        if (count < 1 || count > 16 || length != count * 37 - 1) { llHTTPResponse(request, 400, "invalid"); return; }
        string output;
        integer i;
        for (i = 0; i < count; ++i) {
            integer at = i * 37;
            if (i && llGetSubString(targets, at - 1, at - 1) != ",") { llHTTPResponse(request, 400, "invalid"); return; }
            key avatar = llGetSubString(targets, at, at + 35);
            // A key is true only when it is a valid, non-null UUID.
            if (avatar) {
                list details = llGetObjectDetails(avatar, [OBJECT_POS]);
                // Adjacent avatars can be found within the simulator's documented
                // neighbouring zone. Empty results remain unknown, never zero.
                if (llGetListLength(details) == 1) {
                    vector position = llList2Vector(details, 0);
                    output += ",\"" + (string)avatar + "\"," + (string)position.z;
                }
            } else { llHTTPResponse(request, 400, "invalid"); return; }
        }
        llHTTPResponse(request, 200, "[" + llDeleteSubString(output, 0, 0) + "]");
    } else if (kind == "scripts" || kind == "object") {
        key object = (key)targets;
        if (object == NULL_KEY) { llHTTPResponse(request, 400, "invalid"); return; }
        list fields = [OBJECT_NAME, OBJECT_RUNNING_SCRIPT_COUNT, OBJECT_TOTAL_SCRIPT_COUNT,
            OBJECT_SCRIPT_MEMORY, OBJECT_SCRIPT_TIME, OBJECT_CHARACTER_TIME];
        integer extended = kind == "object";
        if (extended) fields += [OBJECT_DESC, OBJECT_ROOT, OBJECT_PRIM_COUNT, OBJECT_PRIM_EQUIVALENCE,
            OBJECT_TOTAL_INVENTORY_COUNT, OBJECT_VELOCITY, OBJECT_POS, OBJECT_ROT, OBJECT_OMEGA,
            OBJECT_CREATOR, OBJECT_OWNER, OBJECT_LAST_OWNER_ID, OBJECT_REZZER_KEY, OBJECT_GROUP,
            OBJECT_CREATION_TIME, OBJECT_REZ_TIME, OBJECT_PATHFINDING_TYPE, OBJECT_ATTACHED_POINT, OBJECT_TEMP_ATTACHED];
        list details = llGetObjectDetails(object, fields);
        if (llGetListLength(details) != llGetListLength(fields)) { llHTTPResponse(request, 404, "unavailable"); return; }
        // Match the reference's base64 name field: numeric/JSON-shaped names
        // must remain text rather than llList2Json's embedded JSON values.
        details = llListReplaceList(details, [llStringToBase64(llList2String(details, 0))], 0, 0);
        if (extended) {
            details = llListReplaceList(details, [llStringToBase64(llList2String(details, 6))], 6, 6);
            details += [llGetPos(), RAD_TO_DEG * llRot2Euler(llList2Rot(details, 13))];
        }
        llHTTPResponse(request, 200, llList2Json(JSON_ARRAY, [(string)object] + details));
    } else llHTTPResponse(request, 400, "invalid");
}

default {
    // Login restores script globals before attach; revoke old pairing at once.
    on_rez(integer start) { gPaired = FALSE; }
    state_entry() { if (llGetAttached()) initialize(); }
    attach(key wearer) {
        if (wearer) initialize();
        else {
            gPaired = FALSE;
            llListenRemove(gListener);
            llListenRemove(gPoseListener);
            gPoseListener = 0;
            gPosePeers = [];
            llListenRemove(gPortalListener);
            gPortalListener = 0;
            gPortalPeers = [];
            gListener = 0;
            llSetTimerEvent(0.0);
            llReleaseURL(gURL);
            gURL = "";
            gToken = NULL_KEY;
        }
    }
    changed(integer change) {
        if (change & CHANGED_OWNER) llResetScript();
        else if (llGetAttached() && (change & (CHANGED_REGION | CHANGED_REGION_START | CHANGED_TELEPORT))) {
            // Answer the viewer's first probe in the new region at once.
            gNextHello = 0.0;
            requestURL();
        }
    }
    listen(integer channel, string name, key sender, string message) {
        if (channel == PORTAL_CHANNEL) {
            if (!llGetAttached() || !gPaired || llStringLength(message) > 648
                || llGetSubString(message, 0, 7) != "portal2|") return;
            key peer = llGetOwnerKey(sender);
            if (!validKey(peer) || peer == sender || peer == gOwner) return;
            list attached = llGetObjectDetails(sender, [OBJECT_ATTACHED_POINT]);
            if (llGetListLength(attached) != 1 || llList2Integer(attached, 0) <= 0) return;
            integer at = llListFindList(gPortalPeers, [peer]);
            float now = llGetTime();
            if (at >= 0) {
                if (now - llList2Float(gPortalPeers, at + 1) < 0.2) return;
                gPortalPeers = llDeleteSubList(gPortalPeers, at, at + 1);
            }
            if (llGetListLength(gPortalPeers) >= 128) gPortalPeers = llDeleteSubList(gPortalPeers, 0, 1);
            gPortalPeers += [peer, now];
            llOwnerSay("<auroraPortal>" + (string)peer + "|" + llDeleteSubString(message, 0, 7) + "</auroraPortal>");
            return;
        }
        if (channel == POSE_CHANNEL) {
            if (!llGetAttached() || !gPaired || llStringLength(message) > 130) return;
            list fields = llParseStringKeepNulls(message, ["|"], []);
            if (llGetListLength(fields) != 4 || llList2String(fields, 0) != "pose1" || llList2Key(fields, 3) != gOwner) return;
            key nonce = llList2Key(fields, 1);
            string action = llList2String(fields, 2);
            if (!validKey(nonce) || llListFindList(["request", "accept", "refuse", "cancel", "withdraw"], [action]) < 0) return;
            key peer = llGetOwnerKey(sender);
            if (!validKey(peer) || peer == sender || peer == gOwner) return;
            list attached = llGetObjectDetails(sender, [OBJECT_ATTACHED_POINT]);
            if (llGetListLength(attached) != 1 || llList2Integer(attached, 0) <= 0) return;
            if (action == "request") {
                integer at = llListFindList(gPosePeers, [peer]);
                integer now = llGetUnixTime();
                if (at >= 0) {
                    if (now - llList2Integer(gPosePeers, at + 1) < 5) return;
                    gPosePeers = llDeleteSubList(gPosePeers, at, at + 1);
                }
                if (llGetListLength(gPosePeers) >= 64) gPosePeers = llDeleteSubList(gPosePeers, 0, 1);
                gPosePeers += [peer, now];
            }
            // Authenticated owner identity, never a claimed avatar from the payload.
            llOwnerSay("<auroraPose>" + (string)nonce + "|" + action + "|" + (string)peer + "</auroraPose>");
            return;
        }
        if (!llGetAttached() || channel != DISCOVERY_CHANNEL || sender != gOwner
            || message != "auroraHello|" + (string)llGetKey() || llGetTime() < gNextHello) return;
        gNextHello = llGetTime() + 5.0;
        // The grant may arrive after this probe, but never speak before it.
        gPaired = TRUE;
        announce();
    }
    timer() { requestURL(); }
    http_request(key request, string method, string body) {
        if (method == URL_REQUEST_GRANTED) {
            if (request != gURLRequest) { llReleaseURL(body); return; }
            gURL = body;
            if (gPaired) announce();
        } else if (method == URL_REQUEST_DENIED) {
            if (request == gURLRequest) llSetTimerEvent(5.0);
        } else if (method == "POST") {
            if (!gPaired) { llHTTPResponse(request, 403, "unpaired"); return; }
            // llGetHTTPHeader cannot read custom headers: authenticate the
            // bounded body, query|token|kind|targets, read in place.
            if (llStringLength(body) > 768 || llGetSubString(body, 0, 42) != "query|" + (string)gToken + "|") {
                llHTTPResponse(request, 403, "denied"); return;
            }
            body = llDeleteSubString(body, 0, 42);
            integer split = llSubStringIndex(body, "|");
            string targets = llDeleteSubString(body, 0, split);
            body = llGetSubString(body, 0, split - 1);
            if (split < 1 || llSubStringIndex(targets, "|") != -1) { llHTTPResponse(request, 400, "invalid"); return; }
            query(request, body, targets);
        } else llHTTPResponse(request, 405, "method");
    }
}
