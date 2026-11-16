// Aurora Viewer - RLVa self-test relay.
//
// Repeats to its owner, with llOwnerSay, whatever its owner says on its
// private channel: the viewer's RLVa self-test (world/rlva_selftest.rs)
// issues real RLVa commands through it. It listens to its owner only, on a
// negative channel derived from its own key (world::bridge::private_channel),
// which changes at each rez: the script restarts when worn or rezzed.

integer channel()
{
    return -0x10000000 - (integer)("0x" + llGetSubString((string)llGetKey(), 0, 6));
}

default
{
    state_entry()
    {
        llListen(channel(), "", llGetOwner(), "");
    }

    on_rez(integer start)
    {
        llResetScript();
    }

    attach(key id)
    {
        if (id != NULL_KEY)
        {
            llResetScript();
        }
    }

    changed(integer change)
    {
        if (change & CHANGED_OWNER)
        {
            llResetScript();
        }
    }

    listen(integer heard, string name, key id, string message)
    {
        llOwnerSay(message);
    }
}
