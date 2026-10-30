// Aurora bridge IK relay: regional event discovery, no periodic polling.
// Separate Mono memory from movement and read-only bridge services.
string VERSION = "1.40";
string FOLDER = "__FOLDER__";
integer HELLO_CHANNEL = -184735219;
integer CONTROL_CHANNEL = -190018106;
integer IK_CHANNEL = -190018107;
key owner;
key session;
integer started;
integer paired;
float nextHello;
float nextDiscovery;
float window;
integer received;
integer helloListener;
integer controlListener;
integer relayListener;

hello()
{
    if (paired && llGetAttached())
        llOwnerSay("<auroraBridgeIK>" + VERSION + "|" + FOLDER + "|" + (string)session + "</auroraBridgeIK>");
}

initialize()
{
    owner = llGetOwner();
    session = llGenerateKey();
    started = FALSE;
    paired = FALSE;
    nextHello = 0.0;
    nextDiscovery = 0.0;
    window = 0.0;
    received = 0;
    llListenRemove(helloListener);
    llListenRemove(controlListener);
    llListenRemove(relayListener);
    helloListener = llListen(HELLO_CHANNEL, "", owner, "auroraHello|" + (string)llGetKey());
    controlListener = llListen(CONTROL_CHANNEL, "", owner, "");
    relayListener = llListen(IK_CHANNEL, "", NULL_KEY, "");
}

default
{
    state_entry() { initialize(); }
    listen(integer channel, string name, key sender, string message)
    {
        if (!llGetAttached()) return;
        if (channel == HELLO_CHANNEL && sender == owner
            && message == "auroraHello|" + (string)llGetKey() && llGetTime() >= nextHello)
        {
            paired = TRUE;
            nextHello = llGetTime() + 5.0;
            hello();
            return;
        }
        if (!paired) return;
        if (channel == CONTROL_CHANNEL && sender == owner)
        {
            list fields = llParseStringKeepNulls(message, ["|"], []);
            if (llList2String(fields, 0) != "auroraIKRelay" || llList2Key(fields, 1) != session) return;
            string command = llList2String(fields, 2);
            if (command == "discover" && llGetListLength(fields) == 3)
            {
                if (!started || llGetTime() >= nextDiscovery)
                {
                    started = TRUE;
                    nextDiscovery = llGetTime() + 5.0;
                    llRegionSay(IK_CHANNEL, "auroraIKDiscover|" + (string)session);
                }
            }
            else if ((command == "observe" || command == "edit") && started && llGetListLength(fields) == 7)
            {
                key root = llList2Key(fields, 3);
                if (root == NULL_KEY || llList2Key(fields, 4) == NULL_KEY || llList2Key(fields, 5) == NULL_KEY) return;
                string operation = "auroraIKObserve|";
                if (command == "edit") operation = "auroraIKEdit|";
                llRegionSayTo(root, IK_CHANNEL, operation + llDumpList2String(llList2List(fields, 3, 6), "|"));
            }
            return;
        }
        if (channel != IK_CHANNEL || !started || llStringLength(message) > 880) return;
        if (llGetAgentSize(sender) != ZERO_VECTOR) return;
        if (llSubStringIndex(message, "auroraIK/5 ") != 0
            && llSubStringIndex(message, "auroraIKAvailable|") != 0
            && llSubStringIndex(message, "auroraIKState/1 ") != 0) return;
        // Bound forwarding before allocating the envelope. The viewer validates
        // original source ownership, actual seat/wearer and negotiated nonces.
        float now = llGetTime();
        if (now - window >= 1.0) { window = now; received = 0; }
        if (++received > 128) return;
        llOwnerSay("<auroraIKRelay>" + (string)session + "|" + (string)sender + "|"
            + (string)llGetOwnerKey(sender) + "|" + message + "</auroraIKRelay>");
    }
    attach(key wearer) { if (wearer == llGetOwner()) llResetScript(); }
    changed(integer changes)
    {
        if (changes & CHANGED_OWNER) llResetScript();
        else if (changes & (CHANGED_REGION | CHANGED_REGION_START))
        {
            // Retire old-region nonces and pending traffic without forgetting a
            // viewer confirmed during this login. Login/attach always reset it.
            integer confirmed = paired;
            initialize();
            paired = confirmed;
            hello();
        }
    }
    on_rez(integer parameter) { llResetScript(); }
}
