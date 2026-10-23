// Aurora rig contacts: shared observer protocol 5.
// Place one publisher in the ROOT prim of a seat or an owned attachment.
// Observers read definitions; only seated avatars / the wearer are affected.
string RIG = "{\"nodes\":[],\"bindings\":[{\"bone\":\"mWristLeft\",\"root\":\"mTorso\",\"target\":{\"kind\":\"link\",\"object\":\"source\",\"link\":\"Left\"},\"contact\":[0,0.055,-0.015]},{\"bone\":\"mWristRight\",\"root\":\"mTorso\",\"target\":{\"kind\":\"link\",\"object\":\"source\",\"link\":\"Right\"},\"contact\":[0,-0.055,-0.015]}],\"profiles\":[{\"name\":\"drive\",\"bones\":[\"mWristLeft\",\"mWristRight\"]},{\"name\":\"gas_only\",\"bones\":[\"mWristRight\"]}]}";
// Named profiles are part of RIG; animation playback is independent.
string payload;
string profile;
list cycles;
integer bindingCount;
integer validData;
// TRUE waits for the main script's phase messages. FALSE starts immediately.
integer EXTERNAL_CONTROL = TRUE;
integer CONTROL = -190018108;
integer DISCOVERY = -190018107;
integer enabled = TRUE;
integer changedState;
integer disabledMask;
integer revision;
key bundle;
key epoch;
float started;
float preparedAt = -10.0;
list ends;
integer parts;
// viewer, affected avatar (NULL_KEY for explicit authoring), nonce, cached bundle, last request time.
integer STRIDE = 5;
integer MAX_PEERS = 128;
list peers;
list declaredUsers;
list discoveries;
list discoveryQueue;
list queue;
key peer;
key subject;
key nonce;
integer part = -1;
integer sending;
integer sampleEnabled;
integer sampleMask;
key sampleEpoch;
string sampleProfile;
list sampleCycles;
float sampleTime;

integer eligible(key who)
{
    if (llGetAttached() != 0) return who == llGetOwner();
    integer i;
    for (i = 1; i <= llGetNumberOfPrims(); ++i)
        if (llGetLinkKey(i) == who && llGetAgentSize(who) != ZERO_VECTOR) return TRUE;
    return FALSE;
}

integer requestEligible(key who)
{
    // Only the explicit edit command can enqueue a NULL_KEY authoring job.
    if (who == NULL_KEY) return TRUE;
    return eligible(who);
}

list activeUsers()
{
    if (llGetAttached() != 0) return [llGetOwner()];
    list users;
    integer i;
    for (i = 1; i <= llGetNumberOfPrims(); ++i)
    {
        key who = llGetLinkKey(i);
        if (llGetAgentSize(who) != ZERO_VECTOR) users += [who];
    }
    return users;
}

announce()
{
    // Regional events reach listening bridge scripts, not ordinary viewers.
    list users = activeUsers();
    if (validData && llGetListLength(users) <= 16
        && (llGetListLength(users) > 0 || llGetListLength(declaredUsers) > 0))
        llRegionSay(DISCOVERY, "auroraIKAvailable|" + (string)bundle + "|" + llList2Json(JSON_ARRAY, users));
    declaredUsers = users;
}

integer utf8Bytes(string value)
{
    string encoded = llStringToBase64(value);
    integer bytes = llStringLength(encoded) * 3 / 4;
    if (llGetSubString(encoded, -2, -1) == "==") return bytes - 2;
    if (llGetSubString(encoded, -1, -1) == "=") return bytes - 1;
    return bytes;
}

integer profileMask(string selected)
{
    string profiles = llJsonGetValue(payload, ["profiles"]);
    if (profiles == JSON_INVALID || profiles == "[]")
    {
        if (selected == "") return 0;
        return -1;
    }
    integer i;
    integer count = llGetListLength(llJson2List(profiles));
    for (i = 0; i < count; ++i)
        if (llJsonGetValue(payload, ["profiles", i, "name"]) == selected)
        {
            list bones = llJson2List(llJsonGetValue(payload, ["profiles", i, "bones"]));
            string slots = llJsonGetValue(payload, ["profiles", i, "bindings"]);
            list members;
            if (slots != JSON_INVALID) members = llJson2List(slots);
            integer mask = (1 << bindingCount) - 1;
            integer j;
            for (j = 0; j < bindingCount; ++j)
                if ((slots != JSON_INVALID && llListFindList(members, [j]) != -1)
                    || (slots == JSON_INVALID && llListFindList(bones, [llJsonGetValue(payload, ["bindings", j, "bone"])]) != -1))
                    mask = mask & ~(1 << j);
            return mask;
        }
    return -1;
}

applyState(integer nextEnabled, string nextProfile, integer nextMask)
{
    if (enabled == nextEnabled && profile == nextProfile && disabledMask == nextMask) return;
    changedState = TRUE;
    if (nextEnabled && !enabled) { started = llGetTime(); epoch = llGenerateKey(); }
    integer i;
    for (i = 0; i < bindingCount; ++i)
    {
        integer wasActive = enabled && !(disabledMask & (1 << i));
        integer isActive = nextEnabled && !(nextMask & (1 << i));
        if (isActive && !wasActive)
        {
            integer run = llList2Integer(cycles, i);
            if (run == 2147483647) { epoch = llGenerateKey(); run = 0; }
            cycles = llListReplaceList(cycles, [run + 1], i, i);
        }
    }
    enabled = nextEnabled;
    profile = nextProfile;
    disabledMask = nextMask;
}

prepare()
{
    preparedAt = llGetTime();
    validData = FALSE;
    if (utf8Bytes(RIG) > 8192)
    {
        payload = "";
        ends = [];
        llOwnerSay("Aurora IK: the rig definition exceeds 8192 UTF-8 bytes.");
        return;
    }
    // The viewer resolves each root/link pair. Never rebuild target JSON in LSL.
    if (utf8Bytes(RIG) > 8192
        || llGetListLength(llJson2List(llJsonGetValue(RIG, ["bindings"]))) > 16
        || llGetListLength(llJson2List(llJsonGetValue(RIG, ["nodes"]))) > 16
        || llGetListLength(llJson2List(llJsonGetValue(RIG, ["profiles"]))) > 16)
    {
        payload = "";
        ends = [];
        llOwnerSay("Aurora IK: rig data exceeds the supported limits.");
        return;
    }
    validData = TRUE;
    if (bundle != NULL_KEY && payload == RIG) return;
    payload = RIG;
    bundle = llGenerateKey();
    bindingCount = llGetListLength(llJson2List(llJsonGetValue(payload, ["bindings"])));
    cycles = [];
    integer index;
    for (index = 0; index < bindingCount; ++index) cycles += [0];
    if (!EXTERNAL_CONTROL && profile == "" && llJsonGetValue(payload, ["profiles", 0, "name"]) != JSON_INVALID)
        profile = llJsonGetValue(payload, ["profiles", 0, "name"]);
    integer mask = profileMask(profile);
    if (mask < 0) { enabled = FALSE; profile = ""; mask = 65535; }
    disabledMask = mask;
    // Bound actual UTF-8 bytes even if JSON rebuilding unescapes a name.
    // Keep just character boundaries, not another full copy of the payload.
    ends = [];
    integer at = 0;
    integer length = llStringLength(payload);
    while (at < length)
    {
        // Leave room for the relay's original-sender envelope within OwnerSay's
        // 1024 bytes, including sixteen large cycle counters and a long profile.
        integer low = 1;
        integer high = 192;
        if (high > length - at) high = length - at;
        integer take = 1;
        while (low <= high)
        {
            integer middle = (low + high) / 2;
            if (llStringLength(llStringToBase64(llGetSubString(payload, at, at + middle - 1))) <= 256)
            { take = middle; low = middle + 1; }
            else high = middle - 1;
        }
        at += take;
        ends += [at - 1];
    }
    parts = llGetListLength(ends);
}

integer findPeer(key viewer, key avatar)
{
    integer i;
    for (i = 0; i < llGetListLength(peers); i += STRIDE)
        if (llList2Key(peers, i) == viewer && llList2Key(peers, i + 1) == avatar) return i;
    return -1;
}

string job(key viewer, key avatar) { return (string)viewer + "|" + (string)avatar; }

enqueue(key viewer, key avatar)
{
    string value = job(viewer, avatar);
    if (llListFindList(queue, [value]) == -1) queue += [value];
}

prune()
{
    // Scan the linkset once, not once for every observing subscriber.
    list users = activeUsers();
    integer i;
    for (i = llGetListLength(peers) - STRIDE; i >= 0; i -= STRIDE)
        if (llGetTime() - llList2Float(peers, i + 4) > 80.0
            || llGetAgentSize(llGetOwnerKey(llList2Key(peers, i))) == ZERO_VECTOR
            || (llList2Key(peers, i + 1) != NULL_KEY
                && llListFindList(users, [llList2Key(peers, i + 1)]) == -1))
            peers = llDeleteSubList(peers, i, i + STRIDE - 1);
}

string header()
{
    string body = llList2Json(JSON_OBJECT, ["nonce", (string)nonce, "revision", revision,
        "bundle", (string)bundle, "enabled", llList2String([JSON_FALSE, JSON_TRUE], sampleEnabled),
        "epoch", (string)sampleEpoch, "mask", sampleMask, "time", sampleTime, "profile", sampleProfile]);
    body = llJsonSetValue(body, ["cycles"], llList2Json(JSON_ARRAY, sampleCycles));
    key avatar = subject;
    if (avatar == NULL_KEY) avatar = llGetOwnerKey(peer);
    body = llJsonSetValue(body, ["avatar"], (string)avatar);
    return body;
}

emit()
{
    string body = header();
    if (part >= 0)
    {
        body = llJsonSetValue(body, ["part"], (string)part);
        body = llJsonSetValue(body, ["count"], (string)parts);
        integer first;
        if (part > 0) first = llList2Integer(ends, part - 1) + 1;
        body = llJsonSetValue(body, ["data"],
            llStringToBase64(llGetSubString(payload, first, llList2Integer(ends, part))));
    }
    llRegionSayTo(peer, DISCOVERY, "auroraIK/5 " + body);
}

sample(integer index)
{
    peer = llList2Key(peers, index);
    subject = llList2Key(peers, index + 1);
    nonce = llList2Key(peers, index + 2);
    sampleEnabled = enabled && requestEligible(subject);
    sampleMask = disabledMask;
    sampleEpoch = epoch;
    sampleProfile = profile;
    sampleCycles = cycles;
    sampleTime = llGetTime() - started;
    if (sampleTime < 0.0) sampleTime = 0.0;
    ++revision;
}

replyDiscoveries()
{
    list users = activeUsers();
    integer i;
    for (i = llGetListLength(discoveryQueue) - 2; i >= 0; i -= 2)
        if (llGetTime() >= llList2Float(discoveryQueue, i + 1))
        {
            key destination = llList2Key(discoveryQueue, i);
            if (llGetAgentSize(llGetOwnerKey(destination)) != ZERO_VECTOR
                && llGetListLength(users) > 0 && llGetListLength(users) <= 16)
                llRegionSayTo(destination, DISCOVERY,
                    "auroraIKAvailable|" + (string)bundle + "|" + llList2Json(JSON_ARRAY, users));
            discoveryQueue = llDeleteSubList(discoveryQueue, i, i + 1);
        }
}

pump()
{
    if (!validData) { llSetTimerEvent(0.0); sending = FALSE; queue = []; discoveryQueue = []; return; }
    if (llGetListLength(discoveryQueue) != 0) replyDiscoveries();
    if (sending && (findPeer(peer, subject) == -1 || llGetAgentSize(llGetOwnerKey(peer)) == ZERO_VECTOR || !requestEligible(subject)))
    {
        sending = FALSE;
        part = -1;
    }
    if (!sending)
    {
        integer found = FALSE;
        while (!found && llGetListLength(queue) != 0)
        {
            list ids = llParseStringKeepNulls(llList2String(queue, 0), ["|"], []);
            queue = llDeleteSubList(queue, 0, 0);
            integer index = findPeer(llList2Key(ids, 0), llList2Key(ids, 1));
            if (index != -1 && requestEligible(llList2Key(peers, index + 1)))
            {
                sample(index);
                part = -1;
                // Explicit authoring works even when the object has no user.
                if (llList2Key(peers, index + 3) != bundle) part = 0;
                sending = TRUE;
                found = TRUE;
            }
        }
    }
    if (sending)
    {
        emit();
        integer finished = part < 0;
        if (part >= 0)
        {
            ++part;
            if (part >= parts) finished = TRUE;
        }
        if (finished)
        {
            integer index = findPeer(peer, subject);
            if (index != -1 && llList2Key(peers, index + 2) == nonce)
                peers = llDeleteSubList(peers, index, index + STRIDE - 1);
            sending = FALSE;
            part = -1;
        }
    }
    if (sending || llGetListLength(queue) != 0 || llGetListLength(discoveryQueue) != 0) llSetTimerEvent(0.2);
    else llSetTimerEvent(0.0);
}

publish()
{
    // State changes supersede queued/partial snapshots. Release all observers
    // immediately; cached definitions need only this small state heartbeat.
    llSetTimerEvent(0.0);
    sending = FALSE;
    part = -1;
    queue = [];
    prune();
    if (!validData)
    {
        enabled = FALSE;
        if (bundle == NULL_KEY) return;
    }
    // One regional state event replaces per-observer state traffic. Definition
    // bytes still travel only to a requesting bridge, in bounded fragments.
    ++revision;
    string stateBody = llList2Json(JSON_OBJECT, ["revision", revision, "bundle", (string)bundle,
        "epoch", (string)epoch, "enabled", llList2String([JSON_FALSE, JSON_TRUE], enabled),
        "mask", disabledMask, "time", llGetTime() - started, "profile", profile]);
    stateBody = llJsonSetValue(stateBody, ["cycles"], llList2Json(JSON_ARRAY, cycles));
    if (llGetListLength(activeUsers()) != 0 || llGetListLength(declaredUsers) != 0)
        llRegionSay(DISCOVERY, "auroraIKState/1 " + stateBody);
    integer i;
    for (i = 0; i < llGetListLength(peers); i += STRIDE)
    {
        sample(i);
        if (llList2Key(peers, i + 3) != bundle) enqueue(peer, subject);
    }
    if (llGetListLength(queue) != 0 || llGetListLength(discoveryQueue) != 0) llSetTimerEvent(0.2);
    announce();
}

default
{
    state_entry()
    {
        if (llGetLinkNumber() > 1)
        {
            llOwnerSay("Aurora rig contacts: place this script in the ROOT prim.");
            return;
        }
        epoch = llGenerateKey();
        enabled = !EXTERNAL_CONTROL;
        prepare();
        llListen(DISCOVERY, "", NULL_KEY, "");
        if (EXTERNAL_CONTROL)
        {
            enabled = FALSE;
            llMessageLinked(LINK_THIS, CONTROL, "sync", NULL_KEY);
        }
        announce();
    }
    listen(integer channel, string name, key id, string message)
    {
        if (channel != DISCOVERY || llGetAgentSize(llGetOwnerKey(id)) == ZERO_VECTOR) return;
        list fields = llParseStringKeepNulls(message, ["|"], []);
        integer count = llGetListLength(fields);
        if (llList2String(fields, 0) == "auroraIKDiscover" && count == 2)
        {
            integer previous = llListFindList(discoveries, [id]);
            if (previous != -1 && llGetTime() - llList2Float(discoveries, previous + 1) < 5.0) return;
            if (previous != -1) discoveries = llDeleteSubList(discoveries, previous, previous + 1);
            discoveries += [id, llGetTime()];
            if (llGetListLength(discoveries) > 64) discoveries = llDeleteSubList(discoveries, 0, 1);
            list users = activeUsers();
            if (validData && llGetListLength(users) > 0 && llGetListLength(users) <= 16)
            {
                // Spread a region-arrival burst; this timer stops after queued
                // replies/definition fragments, and never polls for viewers.
                if (llGetListLength(discoveryQueue) >= 64 || llGetFreeMemory() < 4096) return;
                discoveryQueue += [id, llGetTime() + llFrand(0.5)];
                llSetTimerEvent(0.2);
            }
            return;
        }
        integer editing = llList2String(fields, 0) == "auroraIKEdit";
        if ((!editing && llList2String(fields, 0) != "auroraIKObserve") || count != 5) return;
        key avatar = llList2Key(fields, 2);
        if (llList2Key(fields, 1) != llGetKey()) return;
        if (editing)
        {
            if (avatar != llGetOwnerKey(id)) return;
            avatar = NULL_KEY;
        }
        else if (!eligible(avatar)) return;
        key request = llList2Key(fields, 3);
        if (request == NULL_KEY) return;
        // Bound name scans with many observers; changed-link events still
        // prepare immediately. A new bundle supersedes all partial transfers.
        if (llGetTime() - preparedAt >= 5.0)
        {
            key previousBundle = bundle;
            prepare();
            if (previousBundle != bundle || !validData) publish();
        }
        if (!validData) return;
        prune();
        integer index = findPeer(id, avatar);
        list row = [id, avatar, request, llList2Key(fields, 4), llGetTime()];
        if (index == -1)
        {
            if (llGetListLength(peers) >= STRIDE * MAX_PEERS || llGetFreeMemory() < 4096) return;
            peers += row;
        }
        else peers = llListReplaceList(peers, row, index, index + STRIDE - 1);
        if (sending && peer == id && subject == avatar) { sending = FALSE; part = -1; }
        enqueue(id, avatar);
        if (!sending) pump();
    }
    timer() { pump(); }
    link_message(integer sender, integer number, string message, key id)
    {
        if (number != CONTROL) return;
        if (sender != llGetLinkNumber()) return;
        changedState = FALSE;
        if (llGetSubString(message, 0, 7) == "profile ")
        {
            string selected = llGetSubString(message, 8, -1);
            integer mask = profileMask(selected);
            if (mask < 0 || llJsonGetValue(payload, ["profiles"]) == JSON_INVALID)
            {
                llOwnerSay("Aurora IK: unknown profile '" + selected + "'.");
                return;
            }
            applyState(TRUE, selected, mask);
        }
        else if (message == "on")
        {
            string selected = profile;
            if (selected == "" && llJsonGetValue(payload, ["profiles", 0, "name"]) != JSON_INVALID)
                selected = llJsonGetValue(payload, ["profiles", 0, "name"]);
            integer mask = profileMask(selected);
            if (mask < 0) return;
            if (selected == profile) mask = mask | disabledMask;
            applyState(TRUE, selected, mask);
        }
        else if (message == "off") applyState(FALSE, profile, disabledMask);
        else
        {
            integer turnOn = llGetSubString(message, 0, 2) == "on ";
            integer turnOff = llGetSubString(message, 0, 3) == "off ";
            if (!turnOn && !turnOff) return;
            string bone;
            if (turnOn) bone = llGetSubString(message, 3, -1);
            else bone = llGetSubString(message, 4, -1);
            integer mask = disabledMask;
            integer i;
            for (i = 0; i < bindingCount; ++i)
                if (llJsonGetValue(payload, ["bindings", i, "bone"]) == bone)
                {
                    if (turnOn) mask = mask & ~(1 << i);
                    else mask = mask | (1 << i);
                }
            // A per-bone command cannot activate outside the named preset.
            integer allowed = profileMask(profile);
            if (allowed < 0) return;
            applyState(enabled, profile, mask | allowed);
        }
        if (changedState) publish();
    }
    changed(integer changes)
    {
        if (changes & CHANGED_OWNER) llResetScript();
        if (changes & (CHANGED_LINK | CHANGED_INVENTORY | CHANGED_REGION | CHANGED_REGION_START))
        {
            prepare();
            publish();
        }
    }
    attach(key who)
    {
        publish();
    }
    on_rez(integer parameter) { llResetScript(); }
}
