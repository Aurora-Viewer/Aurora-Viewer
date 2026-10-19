// Aurora LSL Bridge read-only services v1.28: the Legacy Viewer's getZOffsets
// and getScriptInfo equivalents (fsradar.cpp, fs_resources/EBEDD1D2-*.lsltxt,
// originally LGPL 2.1). A separate script beside the movement controller:
// Mono gives each script its own 64 KB, and a query can never halt Movelock.
// It has its own HTTP-in URL and token, and never touches physics.
string FOLDER = "__FOLDER__";
string VERSION = "1.28";
string gURL;
key gToken;
key gURLRequest;
key gOwner;
integer DISCOVERY_CHANNEL = -184735219;
integer gListener;
integer gPaired;
float gNextHello;

announce() {
    if (gURL != "") llOwnerSay("<auroraBridgeQuery>" + VERSION + "|" + FOLDER + "|" + gURL + "|" + (string)gToken + "</auroraBridgeQuery>");
}
requestURL() {
    llSetTimerEvent(0.0);
    llReleaseURL(gURL);
    gURL = "";
    gToken = llGenerateKey();
    gURLRequest = llRequestSecureURL();
}
initialize() {
    gOwner = llGetOwner();
    llListenRemove(gListener);
    // Same owner-filtered, root-addressed probe as the movement controller.
    gListener = llListen(DISCOVERY_CHANNEL, "", gOwner, "auroraHello|" + (string)llGetKey());
    requestURL();
}
query(key request, string kind, string targets) {
    if (kind == "altitudes") {
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
    state_entry() { if (llGetAttached()) initialize(); }
    attach(key wearer) {
        if (wearer) initialize();
        else {
            gPaired = FALSE;
            llListenRemove(gListener);
            gListener = 0;
            llSetTimerEvent(0.0);
            llReleaseURL(gURL);
            gURL = "";
            gToken = NULL_KEY;
        }
    }
    changed(integer change) {
        if (change & CHANGED_OWNER) llResetScript();
        else if (llGetAttached() && (change & (CHANGED_REGION | CHANGED_REGION_START | CHANGED_TELEPORT))) requestURL();
    }
    listen(integer channel, string name, key sender, string message) {
        if (!llGetAttached() || sender != gOwner || llGetTime() < gNextHello) return;
        gNextHello = llGetTime() + 5.0;
        // Once probed, a replaced URL is announced without waiting for the viewer.
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
