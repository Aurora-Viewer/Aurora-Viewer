// Aurora bridge IK relay: regional event discovery, no periodic polling.
// Separate Mono memory from movement and read-only bridge services.
string VERSION = "1.41";
string FOLDER = "__FOLDER__";
// Both derived as in world/bridge.rs private_channel: the hello from this
// root's key, the viewer's commands from the random session.
integer HELLO_CHANNEL;
integer CONTROL_CHANNEL;
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

integer channelOf(key id)
{
    return -0x10000000 - (integer)("0x" + llGetSubString((string)id, 0, 6));
}

// Ask only the objects avatars sit on, and the wearer's own attachments,
// never the whole region: a seat or worn creation already in use answers
// with its auroraIKAvailable. Others' worn creations show at their next one.
discover()
{
    string message = "auroraIKDiscover|" + (string)session;
    list agents = llGetAgentList(AGENT_LIST_REGION, []);
    list asked;
    integer i;
    for (i = 0; i < llGetListLength(agents) && llGetListLength(asked) < 64; ++i)
    {
        key agent = llList2Key(agents, i);
        key root = llList2Key(llGetObjectDetails(agent, [OBJECT_ROOT]), 0);
        if (root != NULL_KEY && root != agent && llListFindList(asked, [root]) == -1)
        {
            asked += [root];
            llRegionSayTo(root, IK_CHANNEL, message);
        }
    }
    // Addressed to the wearer, it reaches each of their attachments.
    llRegionSayTo(owner, IK_CHANNEL, message);
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
    HELLO_CHANNEL = channelOf(llGetKey());
    CONTROL_CHANNEL = channelOf(session);
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
                    discover();
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
