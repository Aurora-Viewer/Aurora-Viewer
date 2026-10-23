// Optional standalone throttle for the motorcycle's existing engine messages.
// The main game still starts accel_F for every viewer, at the same event.
// Install in the root only when the main script does not embed these helpers.
integer ENGINE_CHANNEL = 17001;
string THROTTLE_NAME = "Right";
float THROTTLE_DEGREES = -28.0;
// Zero chooses the longest local prim axis. Override when using a helper cube.
vector THROTTLE_AXIS = ZERO_VECTOR;
integer throttleLink;
key throttleKey;
rotation throttleRest;
vector throttleAxis;
float throttleStarted = -1.0;
float throttleFrom;
float throttleValue;

float throttleSmooth(float value)
{
    if (value < 0.0) value = 0.0;
    else if (value > 1.0) value = 1.0;
    return value * value * (3.0 - 2.0 * value);
}

throttleApply(float value)
{
    throttleValue = value;
    if (throttleLink > 1 && llGetLinkKey(throttleLink) == throttleKey)
        llSetLinkPrimitiveParamsFast(throttleLink, [PRIM_ROT_LOCAL,
            llAxisAngle2Rot(throttleAxis, THROTTLE_DEGREES * DEG_TO_RAD * value) * throttleRest]);
}

throttleReset()
{
    throttleStarted = -1.0;
    throttleApply(0.0);
}

throttleSetup()
{
    throttleReset();
    throttleLink = 0;
    integer i;
    for (i = 2; i <= llGetNumberOfPrims(); ++i)
        if (llGetLinkName(i) == THROTTLE_NAME && llGetAgentSize(llGetLinkKey(i)) == ZERO_VECTOR)
        {
            if (throttleLink != 0)
            {
                throttleLink = 0;
                llOwnerSay("Throttle: use exactly one child prim named Right.");
                return;
            }
            throttleLink = i;
        }
    if (throttleLink == 0)
    {
        llOwnerSay("Throttle: missing child prim Right.");
        return;
    }
    throttleKey = llGetLinkKey(throttleLink);
    string storage = "aurora.throttle.rest." + (string)throttleKey;
    string saved = llLinksetDataRead(storage);
    if (saved == "")
    {
        throttleRest = llList2Rot(llGetLinkPrimitiveParams(throttleLink, [PRIM_ROT_LOCAL]), 0);
        llLinksetDataWrite(storage, (string)throttleRest);
    }
    else throttleRest = (rotation)saved;
    throttleAxis = THROTTLE_AXIS;
    if (throttleAxis == ZERO_VECTOR)
    {
        vector size = llList2Vector(llGetLinkPrimitiveParams(throttleLink, [PRIM_SIZE]), 0);
        throttleAxis = <1, 0, 0>;
        if (size.y > size.x && size.y >= size.z) throttleAxis = <0, 1, 0>;
        else if (size.z > size.x && size.z > size.y) throttleAxis = <0, 0, 1>;
    }
    throttleAxis = llVecNorm(throttleAxis);
    throttleReset();
}

throttleTap()
{
    throttleFrom = throttleValue;
    throttleStarted = llGetTime();
}

throttleTick()
{
    if (throttleStarted < 0.0) return;
    float elapsed = llGetTime() - throttleStarted;
    if (elapsed < 0.10)
        throttleApply(throttleFrom + (1.0 - throttleFrom) * throttleSmooth(elapsed / 0.10));
    else if (elapsed < 0.16) throttleApply(1.0);
    else if (elapsed < 0.46) throttleApply(1.0 - throttleSmooth((elapsed - 0.16) / 0.30));
    else throttleReset();
}

default
{
    state_entry() { throttleSetup(); }
    link_message(integer sender, integer number, string message, key id)
    {
        if (sender != llGetLinkNumber() || number != ENGINE_CHANNEL) return;
        if (message == "tap") { throttleTap(); llSetTimerEvent(0.05); }
        else if (message == "stop") { throttleReset(); llSetTimerEvent(0.0); }
    }
    timer()
    {
        throttleTick();
        if (throttleStarted < 0.0) llSetTimerEvent(0.0);
    }
    changed(integer changes)
    {
        if (changes & CHANGED_OWNER) { llResetScript(); return; }
        if (changes & CHANGED_LINK) throttleSetup();
    }
    on_rez(integer parameter) { llResetScript(); }
}
