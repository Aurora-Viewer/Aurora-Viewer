// Aurora LSL Bridge v1.20. Movelock physics follow Firestorm's movelockMe
// (EBEDD1D2-A320-43f5-88CF-DD47BBCA5DFB.lsltxt, originally LGPL 2.1).
string FOLDER = "__FOLDER__";
string VERSION = "1.20";
string gURL;
key gToken;
key gURLRequest;
key gOwner;
integer gSequence;
integer gEnabled;
integer gMoving;
integer gSeated;
integer gLocked;
integer gRetryURL;
float gRetryAt;
float gNextHello;
integer DISCOVERY_CHANNEL = -184735219;
integer gDiscoveryListener;
integer gHelloPending;
integer CONTROLS;
integer gMode;
vector gParameters;
float gDepth;
float gSurfaceSpeed;
vector gEntryDrift;
integer gHovering;
vector gHoverAnchor;
integer gJumpAction;
integer gJumpFinished;
integer gJumpRisen;
vector gJumpTarget;
float gJumpPeak;
float gJumpStarted;
integer gPassAction;
integer gPassFinished;
integer gPassAnchored;
vector gPassTarget;
vector gPassDirection;
integer gPassTargetID;
vector gPassOrigin;
integer gPassState;
integer gPassProgress;
integer gPassRemaining;
integer gPassSpeed;
float gMotionCommand;
float gExitStarted;
float gPassStarted;
float gTick;
vector gIntegral;
vector gAnchor;
integer gLevel;
integer gCaptured;
integer gReportedLock;
integer gInitialized;
integer gPermissionsReady;
integer gConnected;
vector gRunForce;
vector gRunDirection;
float gRunIntegral;
float gRunSpeed;
float gRunPower;
integer gRunRestarts;
integer gRunState;
integer gRunPushes;

clearRun(integer reason) {
    if (gRunSpeed > 0.0 && gRunRestarts < 2147483647) ++gRunRestarts;
    if (gRunForce != ZERO_VECTOR) llSetForce(ZERO_VECTOR, FALSE);
    gRunForce = ZERO_VECTOR;
    gRunDirection = ZERO_VECTOR;
    gRunIntegral = 0.0;
    gRunSpeed = 0.0;
    gRunPower = 0.0;
    gRunState = reason;
}

bootStatus() {
    // No endpoint or authentication key in diagnostic status messages.
    llOwnerSay("<auroraBridgeStatus>" + VERSION + "|" + FOLDER + "|" + (string)gPermissionsReady
        + "|" + (string)(gURL != "") + "</auroraBridgeStatus>");
}

announce() {
    if (gHelloPending && gURL != "" && gPermissionsReady && (llGetPermissions() & PERMISSION_TAKE_CONTROLS)
        && llGetPermissionsKey() == gOwner) {
        gHelloPending = FALSE;
        llOwnerSay("<auroraBridge>" + VERSION + "|" + FOLDER + "|" + gURL + "|" + (string)gToken + "</auroraBridge>");
    }
}
discover(integer channel, key sender, string message) {
    // A negative channel is not authentication. Only the wearer's own chat,
    // addressed to this root, may request an announcement. Physics commands
    // still require the current random token and an increasing HTTP sequence.
    if (!gInitialized || !llGetAttached() || channel != DISCOVERY_CHANNEL || sender != gOwner
        || message != "auroraHello|" + (string)llGetKey() || llGetTime() < gNextHello) return;
    gNextHello = llGetTime() + 5.0;
    gHelloPending = TRUE;
    takeControls();
    announce();
    if (gHelloPending) bootStatus();
}

syncControls() {
    // Only swimming intercepts native translation. Run/flight keep the
    // normal controls and animations, as in the supplied movement script.
    // Movelock observes them with llTakeControls(..., TRUE, TRUE), as Firestorm.
    integer capture = !gSeated && (gMode == 3 || gMode == 4 || gMode == 6 || gMode >= 7);
    if (capture != gCaptured && (llGetPermissions() & PERMISSION_TAKE_CONTROLS)) {
        // One motor owns swimming/flight. Still receive control events, so
        // releasing keys stops a delayed HTTP command immediately.
        llTakeControls(CONTROLS, TRUE, !capture);
        gCaptured = capture;
    }
}

syncTimer() {
    if (gMode == 1 || gMode == 5) llSetTimerEvent(0.05);
    else if (gMode || gEnabled) llSetTimerEvent(0.1);
    else if (gRetryURL) llSetTimerEvent(5.0);
    else llSetTimerEvent(0.0);
}
takeControls() {
    if (!gPermissionsReady && llGetAttached() && (llGetPermissions() & PERMISSION_TAKE_CONTROLS)
        && llGetPermissionsKey() == gOwner) {
        llTakeControls(CONTROLS, TRUE, TRUE);
        gPermissionsReady = TRUE;
        syncControls();
    }
}
// Target scheduling and bounded water-entry drift adapted from the
// maintainer-provided movement controller (2026-10-15). No AO/notecard/UI code.
vector bounded(vector value, float maximum) {
    if (llVecMag(value) > maximum) return llVecNorm(value) * maximum;
    return value;
}
rotation movementFacing(integer agentInfo) {
    rotation body = llList2Rot(llGetObjectDetails(gOwner, [OBJECT_ROT]), 0);
    rotation facing = body;
    // The supplied controller steers by camera yaw, retaining camera pitch
    // only for mouselook flight. The simulator already receives these axes
    // through AgentUpdate; no HTTP orientation relay is needed.
    if ((llGetPermissions() & PERMISSION_TRACK_CAMERA) && llGetPermissionsKey() == gOwner) {
        facing = llGetCameraRot();
        if (gMode == 2 && (agentInfo & AGENT_MOUSELOOK)) return facing;
    }
    vector forward = <1,0,0> * facing;
    forward.z = 0;
    // Looking straight up/down has no horizontal heading. Fall back to the
    // avatar instead of rotating a zero vector into an arbitrary direction.
    if (llVecMag(forward) < 0.001) {
        forward = <1,0,0> * body;
        forward.z = 0;
    }
    if (llVecMag(forward) < 0.001) return ZERO_ROTATION;
    return llRotBetween(<1,0,0>, llVecNorm(forward));
}
measurePass() {
    if (!gPassAction) return;
    vector position = llGetPos();
    // Bounded relative measurements only; no coordinates in status messages.
    float progress = (position - gPassOrigin) * gPassDirection;
    if (progress < 0.0) progress = 0.0;
    if (progress > 10000.0) progress = 10000.0;
    gPassProgress = llRound(progress * 100.0);
    float remaining = llVecDist(position, gPassTarget);
    if (remaining > 10000.0) remaining = 10000.0;
    gPassRemaining = llRound(remaining * 100.0);
    float speed = llGetVel() * gPassDirection;
    if (speed > 10000.0) speed = 10000.0;
    if (speed > 0.0 && llRound(speed * 100.0) > gPassSpeed) gPassSpeed = llRound(speed * 100.0);
}
releasePass() {
    if (!gPassAnchored) return;
    gPassAnchored = FALSE;
    // A normal Movelock may already have taken ownership. Never clear its
    // target or vehicle parameters while releasing the temporary passage.
    if (!gLocked) {
        llStopMoveToTarget();
        llSetVehicleType(VEHICLE_TYPE_NONE);
    }
}
releaseMotion() {
    clearRun(0);
    integer finishedPass = gMode == 5 && !gPassFinished;
    if (gMode == 5) {
        measurePass();
        if (gPassState < 3) gPassState = 5;
        gPassFinished = TRUE;
        releasePass();
    }
    if (gMode == 9) gJumpFinished = TRUE;
    if (gPassTargetID) llTargetRemove(gPassTargetID);
    gPassTargetID = 0;
    gMode = 0;
    gParameters = ZERO_VECTOR;
    gEntryDrift = ZERO_VECTOR;
    gSurfaceSpeed = 0.0;
    gHovering = FALSE;
    llSetForce(ZERO_VECTOR, FALSE);
    if (!gLocked) llStopMoveToTarget();
    // Publish the final result even when no subsequent HTTP poll is due.
    // The viewer accepts this private status only from its exact worn root.
    if (finishedPass && gSequence > 0 && llGetAttached()) report();
}
integer passageMode(integer action, vector parameters) {
    float distance = llVecMag(parameters);
    if (action <= 0 || distance < 0.5 || distance > 8.0 || llFabs(parameters.z) > 1.5) return 0;
    if (action != gPassAction) {
        if (gMode == 5) releaseMotion();
        if (gPassTargetID) llTargetRemove(gPassTargetID);
        gPassAction = action;
        gPassFinished = FALSE;
        gPassAnchored = FALSE;
        gPassState = 1;
        gPassProgress = 0;
        gPassRemaining = llRound(distance * 100.0);
        gPassSpeed = 0;
        gPassStarted = llGetTime();
        gPassOrigin = llGetPos();
        gPassTarget = gPassOrigin + parameters;
        gPassDirection = llVecNorm(parameters);
        // Finish close to the far-side goal so the avatar clears the wall.
        // A wide completion radius can release while the body still overlaps it.
        gPassTargetID = llTarget(gPassTarget, 0.1);
    }
    // Retransmission cannot move the goal, renew the lifetime or rearm it.
    if (gPassFinished) return 0;
    return 5;
}
steer(vector position, vector target, float tau) {
    if (llVecDist(position, target) < 0.05) { llMoveToTarget(target, tau); return; }
    list hit = llCastRay(position, target, [RC_REJECT_TYPES, RC_REJECT_AGENTS, RC_MAX_HITS, 1]);
    integer count = llList2Integer(hit, -1);
    if (count < 0) { llStopMoveToTarget(); return; }
    if (count > 0) {
        float clearance = llVecDist(position, llList2Vector(hit, 1)) - 0.35;
        if (clearance <= 0.05) { llStopMoveToTarget(); return; }
        target = position + llVecNorm(target - position) * clearance;
    }
    llMoveToTarget(target, tau);
}
applyMotion() {
    if (gMode != 1) clearRun(0);
    if (gMode != 8 || gLocked) gSurfaceSpeed = 0.0;
    if (!gMode) return;
    integer agentInfo = llGetAgentInfo(gOwner);
    if ((agentInfo & AGENT_SITTING) || llGetTime() - gMotionCommand >= 1.5
        || (gMode == 4 && llGetTime() - gExitStarted >= 0.35)
        || (gMode == 5 && llGetTime() - gPassStarted >= 1.2)
        || (gMode == 9 && llGetTime() - gJumpStarted >= 1.2)) {
        if (gMode == 5 && !(agentInfo & AGENT_SITTING)) {
            // A delayed callback may find the avatar already across. Report
            // the observed result, even if the motor deadline elapsed first.
            vector delta = gPassTarget - llGetPos();
            gPassState = 4;
            if (llVecMag(delta) <= 0.1 || delta * gPassDirection <= 0.0) gPassState = 3;
        }
        releaseMotion();
        if (agentInfo & AGENT_SITTING) gRunState = 9;
        else gRunState = 7;
        return;
    }
    // Movelock retains sole ownership of its validated anchor motor.
    if (gLocked) {
        if (gMode == 5) { gPassState = 6; releaseMotion(); }
        clearRun(3); return;
    }
    vector position = llGetPos();
    if (gMode == 9) {
        if (llVecDist(position, gJumpTarget) < 0.2) { releaseMotion(); return; }
        if (position.z >= gJumpPeak - 0.1 || llGetTime() - gJumpStarted >= 0.65) gJumpRisen = TRUE;
        vector goal = gJumpTarget;
        if (!gJumpRisen) goal = <position.x, position.y, gJumpPeak>;
        steer(position, position + bounded(goal - position, 1.2), 0.16);
        return;
    }
    if (gMode == 5) {
        measurePass();
        vector delta = gPassTarget - position;
        // A fast step can skip the target radius; crossing its plane also
        // completes the action, instead of pulling us back through the wall.
        if (llVecMag(delta) <= 0.1 || delta * gPassDirection <= 0.0) {
            gPassState = 3; releaseMotion(); return;
        }
        if (!gPassAnchored) {
            // The far-side anchor shares Movelock's vehicle parameters.
            // Normal Movelock state/anchor and native controls stay intact.
            gPassAnchored = TRUE;
            llSetVehicleType(VEHICLE_TYPE_SLED);
            llSetVehicleFloatParam(VEHICLE_LINEAR_FRICTION_TIMESCALE, 0.05);
            llSetVehicleFloatParam(VEHICLE_ANGULAR_FRICTION_TIMESCALE, 0.05);
        }
        gPassState = 2;
        // Slightly firmer pull than Movelock's 0.05 s response. Stay above
        // llMoveToTarget's two-physics-frame minimum (2/45 seconds).
        llMoveToTarget(gPassTarget, 0.045);
        return;
    }
    // Release a running target on key-up immediately; collision probes and
    // ordinary steering otherwise run at the timer rate, not every input event.
    if (gMode == 1 && !(gLevel & (CONTROL_FWD | CONTROL_BACK | CONTROL_LEFT | CONTROL_RIGHT))) {
        clearRun(1);
        llStopMoveToTarget(); return;
    }
    vector velocity;
    if (gMode == 1) velocity = llGetVel();
    // Key release/jump must clear the persistent force before the tick-rate
    // limit. It must never survive a jump or native flight transition.
    if (gMode == 1 && ((agentInfo & (AGENT_IN_AIR | AGENT_FLYING))
        || (gLevel & (CONTROL_UP | CONTROL_DOWN)) || llFabs(velocity.z) > 1.5)) {
        if (gLevel & (CONTROL_UP | CONTROL_DOWN)) clearRun(8);
        else clearRun(2);
        llStopMoveToTarget(); return;
    }
    float dt = llGetTime() - gTick;
    float interval = 0.09;
    if (gMode == 1) interval = 0.045;
    if (dt < interval) return;
    gTick = llGetTime();
    if (dt <= 0.0 || dt > 0.2) {
        dt = 0.1;
        if (gMode == 1) dt = 0.05;
    }
    float speed = gParameters.x;
    if (gMode == 1) {
        // x1 is native running, with no damped target at all. Multiply the
        // nominal level-terrain speed, never the previously boosted velocity.
        // wiki.secondlife.com/wiki/Default_Avatar_Movement_Speeds
        if (speed <= 1.0) { clearRun(0); llStopMoveToTarget(); return; }
        if (speed > 5.0) speed = 5.0;
        speed *= 5.13;
    }
    vector direction = <((gLevel & CONTROL_FWD) != 0) - ((gLevel & CONTROL_BACK) != 0),
        ((gLevel & CONTROL_LEFT) != 0) - ((gLevel & CONTROL_RIGHT) != 0), 0>;
    float vertical = ((gLevel & CONTROL_UP) != 0) - ((gLevel & CONTROL_DOWN) != 0);
    rotation facing = movementFacing(agentInfo);
    if (gMode == 2) direction.z = vertical;
    direction = llVecNorm(direction) * facing;
    if (gMode == 1) {
        // Firestorm control() uses llSetForce with llGetMassMKS for avatar
        // flight assist. Keep its available force scale: using legacy mass
        // also reduced the maximum force and slew by 100 in v1.11. Smoothness
        // comes from the separate target/force ramps, not lost motor authority.
        // No vertical force, altitude target or teleport.
        llStopMoveToTarget();
        if (direction == ZERO_VECTOR) { clearRun(1); return; }
        float mass = llGetMassMKS();
        if (mass <= 0.0 || mass > 10000.0) { clearRun(6); return; }
        if (gRunDirection * direction < 0.95) clearRun(0);
        gRunDirection = direction;
        float forward = velocity * direction;
        if (gRunSpeed <= 0.0) {
            gRunSpeed = forward;
            if (gRunSpeed < 5.13) gRunSpeed = 5.13;
            if (gRunSpeed > speed) gRunSpeed = speed;
        }
        // Raise the target by at most 4 m/s each second, including slider
        // changes. Lower targets take effect immediately; never wind up from
        // a large x1 -> x5 speed error in a single script tick.
        gRunSpeed += 4.0 * dt;
        if (gRunSpeed > speed) gRunSpeed = speed;
        float delta = gRunSpeed - forward;
        // Never accumulate thrust against a wall or across a sharp turn.
        if (forward < 0.2) gRunIntegral = 0.0;
        float integral = gRunIntegral + delta * dt * 5.0;
        if (integral < 0.0) integral = 0.0;
        if (integral > 400.0) integral = 400.0;
        float previous = llVecMag(gRunForce) / mass;
        float upward = previous + 20.0 * dt;
        // Back-calculate only the compensation the force ramp can deliver.
        // Freezing all positive integration here would permanently stall the
        // speed below target when native drag needs a larger compensation.
        if (delta >= 0.0) {
            float available = upward - delta * 4.0;
            if (available < gRunIntegral) integral = gRunIntegral;
            else if (integral > available) integral = available;
        }
        gRunIntegral = integral;
        if (gRunIntegral < 0.0) gRunIntegral = 0.0;
        if (gRunIntegral > 400.0) gRunIntegral = 400.0;
        // Keep the learned compensation for native braking near the target;
        // cutting all force at that boundary produces a repeated speed sawtooth.
        // Excess speed unwinds compensation; never apply a reverse brake.
        float acceleration = delta * 4.0 + gRunIntegral;
        if (acceleration < 0.0) acceleration = 0.0;
        if (acceleration > 400.0) acceleration = 400.0;
        if (acceleration > upward) acceleration = upward;
        // Overspeed never gets extra thrust. Clear it immediately instead of
        // holding the previous positive force until another feedback tick.
        if (forward > speed + 0.15) {
            acceleration = 0.0;
            gRunIntegral *= 0.5;
        }
        gRunForce = direction * (acceleration * mass);
        gRunForce.z = 0.0;
        gRunPower = acceleration;
        llSetForce(gRunForce, FALSE);
        gRunState = 4;
        if (forward < speed - 0.1) gRunState = 5;
        if (gRunSpeed < speed - 0.05) gRunState = 10;
        else if (acceleration >= 399.9 && forward < speed - 0.1) gRunState = 11;
        if (acceleration > 0.0 && gRunPushes < 2147483647) ++gRunPushes;
        return;
    }
    if (gMode == 7 || gMode == 8 || gMode == 10) {
        vector goal = position + direction * speed * 0.2;
        goal.z = llWater(ZERO_VECTOR) + gParameters.z;
        if (gMode == 8) {
            // Surface return is gentler than horizontal swimming: ramp its
            // vertical command and ease off before reaching flotation.
            float error = goal.z - position.z;
            float limit = speed;
            if (limit > 2.0) limit = 2.0;
            float wanted = error * 1.2;
            if (wanted > limit) wanted = limit;
            if (wanted < -limit) wanted = -limit;
            float change = 1.5 * dt;
            if (wanted > gSurfaceSpeed + change) wanted = gSurfaceSpeed + change;
            if (wanted < gSurfaceSpeed - change) wanted = gSurfaceSpeed - change;
            gSurfaceSpeed = wanted;
            float stepZ = wanted * 0.2;
            // Entry momentum can carry the body past the goal. Never place
            // the next vertical target beyond it while the ramp catches up.
            if (error >= 0.0) {
                if (stepZ < 0.0) stepZ = 0.0;
                if (stepZ > error) stepZ = error;
            } else {
                if (stepZ > 0.0) stepZ = 0.0;
                if (stepZ < error) stepZ = error;
            }
            goal.z = position.z + stepZ;
        }
        float maximum = speed * 0.2;
        if (maximum > 1.6) maximum = 1.6;
        steer(position, position + bounded(goal - position, maximum), 0.2);
        return;
    }
    if (gMode == 3 || gMode == 6 || gMode == 4) {
        float water = llWater(ZERO_VECTOR);
        if (water - llGround(ZERO_VECTOR) <= gParameters.y * 0.55 && gMode != 4) {
            releaseMotion(); return;
        }
        vector target = position + direction * speed * 0.2;
        if (gMode == 3) {
            float depth = gParameters.y * 0.5 - 0.4;
            if (depth < 0.25) depth = 0.25;
            if (depth > 0.8) depth = 0.8;
            target.z = water - depth + gParameters.z;
        } else if (gMode == 6) {
            float ascent = speed;
            if (ascent > 4.0) ascent = 4.0;
            gDepth += vertical * ascent * dt;
            // Keep depth anchored at rest, without accumulating unreachable goals.
            if (gDepth > position.z + 0.75) gDepth = position.z + 0.75;
            if (gDepth < position.z - 0.75) gDepth = position.z - 0.75;
            if (gDepth > water + gParameters.z - 0.25) gDepth = water + gParameters.z - 0.25;
            target.z = gDepth;
        } else target.z = position.z + gParameters.z * 0.2;
        float ceiling = target.z;
        if (gMode == 6) ceiling = water + gParameters.z - 0.25;
        target += gEntryDrift;
        gEntryDrift *= 0.5;
        if (gMode != 4 && target.z > ceiling) target.z = ceiling;
        steer(position, position + bounded(target - position, 1.6), 0.2);
        return;
    }
    if (direction == ZERO_VECTOR) {
        if (gMode == 2) {
            if (!gHovering) { gHoverAnchor = position; gHovering = TRUE; }
            llMoveToTarget(gHoverAnchor, 0.2);
        } else llStopMoveToTarget();
        return;
    }
    gHovering = FALSE;
    // Recompute from current position and actual controls at every tick. A
    // delayed viewer message cannot pull us back toward an old world vector.
    vector target = position + direction * speed * 0.2;
    steer(position, target, 0.2);
}

string status() {
    integer held = gLocked && llVecDist(llGetPos(), gAnchor) < 0.35 && llVecMag(llGetVel()) < 0.5;
    gReportedLock = held;
    vector velocity = llGetVel();
    velocity.z = 0.0;
    integer target;
    if (gMode == 1) target = llRound(gParameters.x * 5.13 * 100.0);
    return (string)gSequence + "|" + (string)gEnabled + "|" + (string)held + "|" + (string)gMoving + "|" + (string)gSeated
        + "|" + (string)gMode + "|" + (string)gRunState + "|" + (string)llRound(llVecMag(velocity) * 100.0)
        + "|" + (string)target + "|" + (string)gRunPushes
        + "|" + (string)llRound(gRunPower * 1000.0) + "|" + (string)llRound(gRunSpeed * 100.0)
        + "|" + (string)gRunRestarts
        + "|" + (string)gPassAction + "|" + (string)gPassState + "|" + (string)gPassProgress
        // Last field retains the v1.15 impulse counter slot; no impulses in
        // the remote-anchor motor. Keep the bounded status wire-compatible.
        + "|" + (string)gPassRemaining + "|" + (string)gPassSpeed + "|0";
}
report() { llOwnerSay("<auroraMoveLock>" + status() + "</auroraMoveLock>"); }
setLock(integer yes) {
    if (yes == gLocked) return;
    gLocked = yes;
    if (yes) {
        llSetForce(ZERO_VECTOR, FALSE);
        gIntegral = ZERO_VECTOR;
        gAnchor = llGetPos();
        llSetVehicleType(VEHICLE_TYPE_SLED);
        llSetVehicleFloatParam(VEHICLE_LINEAR_FRICTION_TIMESCALE, 0.05);
        llSetVehicleFloatParam(VEHICLE_ANGULAR_FRICTION_TIMESCALE, 0.05);
        llMoveToTarget(gAnchor, 0.05);
    } else {
        llStopMoveToTarget();
        llSetVehicleType(VEHICLE_TYPE_NONE);
    }
}
syncLock() {
    gSeated = (llGetAgentInfo(gOwner) & AGENT_SITTING) != 0;
    // Only simulator control events detect voluntary movement. Velocity,
    // bumps, buoyancy and viewer HTTP motion hints cannot release the anchor.
    setLock(gEnabled && !gSeated && !gMoving);
    if (gLocked) llMoveToTarget(gAnchor, 0.05);
    syncControls();
}
// Normal Movelock remains control-driven. Explicit water destinations own the
// motor only until completion/cancellation or the existing command timeout.
syncMotionLock() {
    if (gMode == 7 || gMode == 8 || gMode == 9 || gMode == 10) {
        gSeated = (llGetAgentInfo(gOwner) & AGENT_SITTING) != 0;
        setLock(FALSE);
        syncControls();
    } else syncLock();
}
requestURL() {
    // Notify an already paired viewer once when the simulator replaces its
    // endpoint. Otherwise wait for discovery, even on an unsupported viewer.
    if (gConnected) gHelloPending = TRUE;
    llReleaseURL(gURL);
    gURL = "";
    gToken = llGenerateKey();
    gSequence = 0;
    gRetryURL = FALSE;
    gConnected = FALSE;
    gURLRequest = llRequestSecureURL();
    syncTimer();
}
initialize() {
    // state_entry and attach can both arrive for the same rez. A second
    // llRequestPermissions clears the first grant while its event is queued.
    if (gInitialized) return;
    gInitialized = TRUE;
    gPermissionsReady = FALSE;
    gHelloPending = FALSE;
    gNextHello = 0.0;
    CONTROLS = CONTROL_FWD | CONTROL_BACK | CONTROL_LEFT | CONTROL_RIGHT | CONTROL_UP | CONTROL_DOWN;
    gOwner = llGetOwner();
    llListenRemove(gDiscoveryListener);
    gDiscoveryListener = llListen(DISCOVERY_CHANNEL, "", gOwner, "auroraHello|" + (string)llGetKey());
    gEnabled = FALSE;
    gMoving = FALSE;
    gLevel = 0;
    gCaptured = FALSE;
    gReportedLock = FALSE;
    releaseMotion();
    // A reset must remove any physics left by the previous script instance.
    llStopMoveToTarget();
    llSetVehicleType(VEHICLE_TYPE_NONE);
    gLocked = FALSE;
    llSetLinkPrimitiveParamsFast(LINK_THIS, [PRIM_SIZE, <0.01,0.01,0.01>, PRIM_COLOR, ALL_SIDES, <1,1,1>, 0.0]);
    // Request both together: a later request would replace the controls grant.
    // Camera tracking is optional; Movelock still needs only TAKE_CONTROLS.
    llRequestPermissions(gOwner, PERMISSION_TAKE_CONTROLS | PERMISSION_TRACK_CAMERA);
    requestURL();
}
// Read-only Firestorm getZOffsets/getScriptInfo equivalents. Authentication
// is shared with control requests, but these never touch motor state or timers.
query(key request, string kind, string targets) {
    if (kind == "altitudes") {
        list ids = llParseStringKeepNulls(targets, [","], []);
        integer count = llGetListLength(ids);
        if (count < 1 || count > 32) { llHTTPResponse(request, 400, "invalid"); return; }
        list output;
        integer i;
        for (i = 0; i < count; ++i) {
            key avatar = llList2Key(ids, i);
            if (avatar == NULL_KEY) { llHTTPResponse(request, 400, "invalid"); return; }
            list details = llGetObjectDetails(avatar, [OBJECT_POS]);
            // Adjacent avatars can be found within the simulator's documented
            // neighbouring zone. Empty results remain unknown, never zero.
            if (llGetListLength(details) == 1) {
                vector position = llList2Vector(details, 0);
                output += [(string)avatar, position.z];
            }
        }
        llHTTPResponse(request, 200, llList2Json(JSON_ARRAY, output));
    } else if (kind == "scripts") {
        key object = (key)targets;
        if (object == NULL_KEY) { llHTTPResponse(request, 400, "invalid"); return; }
        list details = llGetObjectDetails(object, [OBJECT_NAME, OBJECT_RUNNING_SCRIPT_COUNT,
            OBJECT_TOTAL_SCRIPT_COUNT, OBJECT_SCRIPT_MEMORY, OBJECT_SCRIPT_TIME, OBJECT_CHARACTER_TIME]);
        if (llGetListLength(details) != 6) { llHTTPResponse(request, 404, "unavailable"); return; }
        // Match Firestorm's base64 name field: numeric/JSON-shaped names
        // must remain text rather than llList2Json's embedded JSON values.
        details = llListReplaceList(details, [llStringToBase64(llList2String(details, 0))], 0, 0);
        llHTTPResponse(request, 200, llList2Json(JSON_ARRAY, [(string)object] + details));
    } else llHTTPResponse(request, 400, "invalid");
}

default {
    state_entry() { if (llGetAttached()) initialize(); }
    attach(key wearer) {
        if (wearer) initialize();
        else {
            gInitialized = FALSE;
            gPermissionsReady = FALSE;
            gEnabled = FALSE;
            gRetryURL = FALSE;
            gConnected = FALSE;
            gHelloPending = FALSE;
            llListenRemove(gDiscoveryListener);
            gDiscoveryListener = 0;
            llSetTimerEvent(0);
            setLock(FALSE);
            releaseMotion();
            llReleaseControls();
            llReleaseURL(gURL);
            gURL = "";
            gURLRequest = NULL_KEY;
            gToken = NULL_KEY;
        }
    }
    on_rez(integer start) {
        // on_rez precedes attach. llResetScript here erases that queued
        // attach event before the simulator has finished attaching the prim.
        // Preserve the queue; attach starts a fresh URL/permission generation.
        gInitialized = FALSE;
        gPermissionsReady = FALSE;
        gConnected = FALSE;
        gHelloPending = FALSE;
        llListenRemove(gDiscoveryListener);
        gDiscoveryListener = 0;
        llSetTimerEvent(0.0);
    }
    run_time_permissions(integer granted) {
        if (llGetAttached() && (granted & PERMISSION_TAKE_CONTROLS)
            && (llGetPermissions() & PERMISSION_TAKE_CONTROLS) && llGetPermissionsKey() == gOwner) {
            takeControls();
            announce();
        }
    }
    listen(integer channel, string name, key sender, string message) {
        discover(channel, sender, message);
    }
    control(key owner, integer level, integer edge) {
        gLevel = level;
        if (edge) gIntegral = ZERO_VECTOR;
        integer moving = (level & CONTROLS) != 0;
        if (moving != gMoving) {
            gMoving = moving;
            if (gEnabled) { syncMotionLock(); report(); }
        }
        if (gMode) applyMotion();
        syncControls();
    }
    at_target(integer targetID, vector targetPosition, vector currentPosition) {
        // The event can be delayed: recheck the current position rather than
        // declaring success solely because it entered the radius earlier.
        if (gMode == 5 && targetID == gPassTargetID) applyMotion();
    }
    not_at_target() {
        if (gMode == 5) {
            applyMotion();
            syncTimer();
        }
    }
    changed(integer change) {
        if (change & CHANGED_OWNER) llResetScript();
        else if (gInitialized && llGetAttached() && (change & (CHANGED_REGION | CHANGED_REGION_START | CHANGED_TELEPORT))) {
            setLock(FALSE);
            releaseMotion();
            syncControls();
            llSetTimerEvent(0);
            requestURL();
            syncMotionLock();
        }
    }
    timer() {
        if (gRetryURL && llGetTime() >= gRetryAt) requestURL();
        applyMotion();
        if (gEnabled) {
            integer seated = gSeated;
            integer locked = gLocked;
            syncMotionLock();
            integer held = gLocked && llVecDist(llGetPos(), gAnchor) < 0.35 && llVecMag(llGetVel()) < 0.5;
            if (seated != gSeated || locked != gLocked || held != gReportedLock) report();
        }
        syncTimer();
        syncControls();
    }
    http_request(key request, string method, string body) {
        if (method == URL_REQUEST_GRANTED) {
            if (request != gURLRequest) { llReleaseURL(body); return; }
            gURL = body;
            syncTimer();
            announce();
        } else if (method == URL_REQUEST_DENIED) {
            if (request != gURLRequest) return;
            gRetryURL = TRUE;
            gRetryAt = llGetTime() + 5.0;
            syncTimer();
            if (gHelloPending) {
                gHelloPending = FALSE;
                llOwnerSay("<auroraBridgeError>url</auroraBridgeError>");
            }
        } else if (method == "POST") {
            // llGetHTTPHeader cannot read custom headers: authenticate the
            // bounded POST body instead (wiki.secondlife.com/wiki/LlGetHTTPHeader).
            list parts = llParseStringKeepNulls(body, ["|"], []);
            if (llList2String(parts, 0) == "query") {
                if (llStringLength(body) > 1536 || llGetListLength(parts) != 4
                    || llList2String(parts, 1) != (string)gToken) {
                    llHTTPResponse(request, 403, "denied"); return;
                }
                query(request, llList2String(parts, 2), llList2String(parts, 3));
                return;
            }
            if (llStringLength(body) > 256 || llGetListLength(parts) != 10 || llList2String(parts, 1) != (string)gToken) {
                llHTTPResponse(request, 403, "denied"); return;
            }
            if ((llGetPermissions() & PERMISSION_TAKE_CONTROLS) == 0) {
                llHTTPResponse(request, 409, "permissions"); return;
            }
            integer sequence = (integer)llList2String(parts, 2);
            if (llList2String(parts, 0) != "lock" || sequence <= 0) {
                llHTTPResponse(request, 400, "invalid"); return;
            }
            gConnected = TRUE;
            if (sequence > gSequence) {
                gSequence = sequence;
                gEnabled = llList2Integer(parts, 3) != 0;
                // Field 4 is retained for wire compatibility; movement is always free.
                integer mode = llList2Integer(parts, 5);
                vector parameters = <llList2Float(parts, 6), llList2Float(parts, 7), llList2Float(parts, 8)>;
                integer action = llList2Integer(parts, 9);
                integer previousMode = gMode;
                if (mode == 9) {
                    if (action != gJumpAction) {
                        gJumpAction = action;
                        gJumpFinished = FALSE;
                        gJumpRisen = FALSE;
                        gJumpStarted = llGetTime();
                        gJumpTarget = llGetPos() + parameters;
                        gJumpPeak = gJumpTarget.z + 0.25;
                    }
                    if (gJumpFinished || action <= 0 || llVecMag(parameters) > 4.0
                        || parameters.z < 0.1 || parameters.z > 3.5) mode = 0;
                }
                if (mode == 5) mode = passageMode(action, parameters);
                if (mode != previousMode) {
                    clearRun(0);
                    gSurfaceSpeed = 0.0;
                    if (previousMode == 5 && mode != 5) {
                        releaseMotion();
                    }
                    if (!gLocked) llStopMoveToTarget();
                    gHovering = FALSE;
                    if (mode == 3 || mode == 6) {
                        vector position = llGetPos();
                        gDepth = position.z;
                        if (previousMode != 3 && previousMode != 6)
                            gEntryDrift = bounded(llGetVel() * 0.2, 0.75);
                    }
                    if (mode == 4) gExitStarted = llGetTime();
                }
                gMode = mode;
                gParameters = parameters;
                if (gMode < 0 || gMode > 10) releaseMotion();
                gMotionCommand = llGetTime();
                syncMotionLock();
                if (gMode) applyMotion();
                else releaseMotion();
                syncTimer();
            }
            llHTTPResponse(request, 200, status());
            syncTimer();
        } else if (method == "GET" || method == "PUT" || method == "DELETE") llHTTPResponse(request, 405, "method");
    }
}
