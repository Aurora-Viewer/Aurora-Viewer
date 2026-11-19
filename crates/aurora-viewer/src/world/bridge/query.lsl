// Aurora LSL Bridge query and posing-consent services v1.44: the Legacy Viewer's getZOffsets
// and getScriptInfo equivalents (fsradar.cpp, fs_resources/EBEDD1D2-*.lsltxt,
// originally LGPL 2.1). A separate script beside the movement controller:
// Mono gives each script its own 64 KB, and a query can never halt Movelock.
// It has its own HTTP-in URL and token, and never touches physics.
string FOLDER = "__FOLDER__";
string VERSION = "1.44";
string gURL;
key gToken;
key gURLRequest;
key gOwner;
integer DISCOVERY_CHANNEL;
integer gListener;
integer gPoseListener;
// Posing consent is asked as any posing HUD asks it: a dialog to the resident,
// answered with its buttons or with "stop" in local chat, whatever viewer they
// use. Nothing goes to another viewer (Second Life's shared experience policy).
integer POSE_DIALOG_CHANNEL;
integer STOP_CHANNEL = 2525;
integer gStopListener;
list gAsked;
list gGranted;
string gYes;
string gNo;
string gStop;
string gGrantedText;
string gStopped;
integer gPaired;
float gNextHello;

// Forget every consent: after a reset, a detach or a region change.
forgetPoses() {
    llListenRemove(gPoseListener);
    llListenRemove(gStopListener);
    gPoseListener = 0;
    gStopListener = 0;
    gAsked = [];
    gGranted = [];
}
forgetPose(key resident) {
    // [resident, nonce] pairs: a resident's key is never one of our nonces.
    integer at = llListFindList(gAsked, [resident]);
    if (at >= 0) gAsked = llDeleteSubList(gAsked, at, at + 1);
    at = llListFindList(gGranted, [resident]);
    if (at >= 0) gGranted = llDeleteSubList(gGranted, at, at + 1);
}
// Listen only while someone is asked or has accepted.
poseListeners() {
    integer granted = llGetListLength(gGranted);
    integer any = llGetListLength(gAsked) + granted;
    if (any && !gPoseListener) gPoseListener = llListen(POSE_DIALOG_CHANNEL, "", NULL_KEY, "");
    if (!any && gPoseListener) { llListenRemove(gPoseListener); gPoseListener = 0; }
    if (granted && !gStopListener) gStopListener = llListen(STOP_CHANNEL, "", NULL_KEY, "");
    if (!granted && gStopListener) { llListenRemove(gStopListener); gStopListener = 0; }
}

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
    // Same owner-filtered, root-addressed probe and channel as the movement controller.
    DISCOVERY_CHANNEL = -0x10000000 - (integer)("0x" + llGetSubString((string)llGetKey(), 0, 6));
    gListener = llListen(DISCOVERY_CHANNEL, "", gOwner, "auroraHello|" + (string)llGetKey());
    POSE_DIALOG_CHANNEL = -0x40000000 - (integer)("0x" + llGetSubString((string)llGetKey(), 0, 6));
    forgetPoses();
    requestURL();
}
query(key request, string kind, string targets) {
    if (kind == "pose") {
        // ask: the consent dialog, its texts in base64 in the poser's language;
        // forget: the request or the consent ended on the viewer's side.
        list fields = llParseStringKeepNulls(targets, [","], []);
        string action = llList2String(fields, 0);
        key nonce = llList2Key(fields, 1);
        key target = llList2Key(fields, 2);
        integer count = llGetListLength(fields);
        if (!validKey(nonce) || !validKey(target) || target == gOwner
            || !((action == "ask" && count == 9) || (action == "forget" && count == 3))) {
            llHTTPResponse(request, 400, "invalid"); return;
        }
        forgetPose(target);
        if (action == "ask") {
            gYes = llBase64ToString(llList2String(fields, 4));
            gNo = llBase64ToString(llList2String(fields, 5));
            gGrantedText = llBase64ToString(llList2String(fields, 6));
            gStop = llBase64ToString(llList2String(fields, 7));
            gStopped = llBase64ToString(llList2String(fields, 8));
            if (llGetListLength(gAsked) >= 64) gAsked = llDeleteSubList(gAsked, 0, 1);
            gAsked += [target, nonce];
            llDialog(target, llBase64ToString(llList2String(fields, 3)), [gYes, gNo], POSE_DIALOG_CHANNEL);
        }
        poseListeners();
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
            forgetPoses();
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
            // A consent holds within one region.
            forgetPoses();
            requestURL();
        }
    }
    listen(integer channel, string name, key sender, string message) {
        if (channel == POSE_DIALOG_CHANNEL || channel == STOP_CHANNEL) {
            // The resident's own answer: dialog buttons and local chat come
            // from their avatar's key, which no object can speak as.
            if (!llGetAttached() || !gPaired) return;
            integer at = llListFindList(gAsked, [sender]);
            if (channel == POSE_DIALOG_CHANNEL && at >= 0 && (message == gYes || message == gNo)) {
                key asked = llList2Key(gAsked, at + 1);
                gAsked = llDeleteSubList(gAsked, at, at + 1);
                string answer = "refuse";
                if (message == gYes) {
                    answer = "accept";
                    if (llGetListLength(gGranted) >= 64) gGranted = llDeleteSubList(gGranted, 0, 1);
                    gGranted += [sender, asked];
                }
                poseListeners();
                llOwnerSay("<auroraPose>" + (string)asked + "|" + answer + "|" + (string)sender + "</auroraPose>");
                if (answer == "accept") {
                    // How to stop, with a Stop button for when chat is out of
                    // range, and the same in an IM, kept in their history.
                    // llInstantMessage sleeps the script 2 s: last, after the
                    // viewer has its answer.
                    llDialog(sender, gGrantedText, [gStop], POSE_DIALOG_CHANNEL);
                    llInstantMessage(sender, gGrantedText);
                }
                return;
            }
            at = llListFindList(gGranted, [sender]);
            if (at < 0) return;
            if (channel == POSE_DIALOG_CHANNEL && message != gStop) return;
            if (channel == STOP_CHANNEL && llToLower(llStringTrim(message, STRING_TRIM)) != "stop") return;
            key granted = llList2Key(gGranted, at + 1);
            gGranted = llDeleteSubList(gGranted, at, at + 1);
            poseListeners();
            if (gStopped != "") llRegionSayTo(sender, 0, gStopped);
            llOwnerSay("<auroraPose>" + (string)granted + "|withdraw|" + (string)sender + "</auroraPose>");
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
            if (llStringLength(body) > 1600 || llGetSubString(body, 0, 42) != "query|" + (string)gToken + "|") {
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
