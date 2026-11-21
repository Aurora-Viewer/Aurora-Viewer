//! The Legacy Viewer's behaviour dictionary, in registration order: the list
//! @getcommand walks. Port of RlvBehaviourDictionary::RlvBehaviourDictionary
//! and getCommands (indra/newview/rlvhelper.cpp, originally LGPL 2.1).
//!
//! Only dictionary entries are listed: the @setdebug_/@getdebug_ and
//! @setenv_/@getenv_ names, @setrot and the local modifiers
//! (`setoverlay_alpha`, `setsphere_mode`…) are handled outside it and never
//! appear in a @getcommand reply.

/// RLV_TYPE_ADDREM, RLV_TYPE_FORCE and RLV_TYPE_REPLY (indra/newview/rlvdefines.h).
pub(super) const ADD: u8 = 0b0011;
pub(super) const FORCE: u8 = 0b0100;
pub(super) const REPLY: u8 = 0b1000;
/// RlvBehaviourInfo::BHVR_STRICT: also listed with "_sec".
const STRICT: u8 = 0b1_0000;
/// RlvBehaviourInfo::BHVR_EXPERIMENTAL: left out by addEntry when
/// RLVaExperimentalCommands is off.
const EXPERIMENTAL: u8 = 0b10_0000;

/// The entries of RlvBehaviourDictionary's constructor, one per addEntry, with
/// their parameter type and flags (a processor's type comes from its base:
/// RlvBehaviourProcessor and the toggles are ADDREM, RlvForceProcessor FORCE,
/// RlvReplyProcessor REPLY).
const ENTRIES: &[(&str, u8)] = &[
    ("acceptpermission", ADD),
    ("accepttp", ADD | STRICT),
    ("accepttprequest", ADD | STRICT),
    ("addattach", ADD),
    ("addoutfit", ADD),
    ("allowidle", ADD | EXPERIMENTAL),
    ("alwaysrun", ADD),
    ("attachthis", ADD),
    ("attachallthis", ADD),
    ("attachthis_except", ADD),
    ("attachallthis_except", ADD),
    ("buy", ADD),
    ("chatwhisper", ADD),
    ("chatnormal", ADD),
    ("chatshout", ADD),
    ("detach", ADD),
    ("detachthis", ADD),
    ("detachallthis", ADD),
    ("detachthis_except", ADD),
    ("detachallthis_except", ADD),
    ("edit", ADD),
    ("editattach", ADD),
    ("editobj", ADD),
    ("editworld", ADD),
    ("viewtransparent", ADD | EXPERIMENTAL),
    ("viewwireframe", ADD | EXPERIMENTAL),
    ("emote", ADD),
    ("fartouch", ADD),
    ("fly", ADD),
    ("interact", ADD),
    ("jump", ADD),
    ("notify", ADD),
    ("pay", ADD),
    ("permissive", ADD),
    ("recvchat", ADD | STRICT),
    ("recvchatfrom", ADD | STRICT),
    ("recvemote", ADD | STRICT),
    ("recvemotefrom", ADD | STRICT),
    ("recvim", ADD | STRICT),
    ("recvimfrom", ADD | STRICT),
    ("redirchat", ADD),
    ("rediremote", ADD),
    ("remattach", ADD),
    ("remoutfit", ADD),
    ("rez", ADD),
    ("sendchannel", ADD | STRICT),
    ("sendchannel_except", ADD | STRICT | EXPERIMENTAL),
    ("sendchat", ADD),
    ("sendim", ADD | STRICT),
    ("sendimto", ADD | STRICT),
    ("sendgesture", ADD | EXPERIMENTAL),
    ("setdebug", ADD),
    ("setenv", ADD),
    ("setgroup", ADD),
    ("share", ADD | STRICT),
    ("sharedunwear", ADD),
    ("sharedwear", ADD),
    ("showhovertext", ADD),
    ("showhovertextall", ADD),
    ("showhovertexthud", ADD),
    ("showhovertextworld", ADD),
    ("showinv", ADD),
    ("showloc", ADD),
    ("showminimap", ADD),
    ("shownames", ADD | STRICT),
    ("shownametags", ADD | STRICT),
    ("shownearby", ADD | EXPERIMENTAL),
    ("showself", ADD | EXPERIMENTAL),
    ("showselfhead", ADD | EXPERIMENTAL),
    ("showworldmap", ADD),
    ("sit", ADD),
    ("sittp", ADD),
    ("standtp", ADD),
    ("startim", ADD | STRICT),
    ("startimto", ADD | STRICT),
    ("temprun", ADD),
    ("touchall", ADD),
    ("touchattach", ADD),
    ("touchattachother", ADD),
    ("touchattachself", ADD),
    ("touchfar", ADD),
    ("touchhud", ADD),
    ("touchme", ADD),
    ("touchthis", ADD),
    ("touchworld", ADD),
    ("tplm", ADD),
    ("tploc", ADD),
    ("tplocal", ADD | EXPERIMENTAL),
    ("tplure", ADD | STRICT),
    ("tprequest", ADD | STRICT),
    ("unsharedunwear", ADD),
    ("unsharedwear", ADD),
    ("unsit", ADD),
    ("viewnote", ADD),
    ("viewscript", ADD),
    ("viewtexture", ADD),
    ("setcam", ADD),
    ("setcam_avdist", ADD),
    ("setcam_avdistmin", ADD | EXPERIMENTAL),
    ("setcam_avdistmax", ADD | EXPERIMENTAL),
    ("setcam_origindistmin", ADD | EXPERIMENTAL),
    ("setcam_origindistmax", ADD | EXPERIMENTAL),
    ("setcam_eyeoffset", ADD),
    ("setcam_eyeoffsetscale", ADD),
    ("setcam_focusoffset", ADD),
    ("setcam_fovmin", ADD),
    ("setcam_fovmax", ADD),
    ("setcam_mouselook", ADD),
    ("setcam_textures", ADD),
    ("setcam_unlock", ADD),
    ("camavdist", ADD),
    ("camdistmin", ADD),
    ("camdistmax", ADD),
    ("camtextures", ADD),
    ("camzoommin", ADD),
    ("camzoommax", ADD),
    ("camunlock", ADD),
    ("setoverlay", ADD),
    ("setoverlay_touch", ADD),
    ("setoverlay_tween", FORCE),
    ("setsphere", ADD),
    ("attach", FORCE),
    ("attachall", FORCE),
    ("attachover", FORCE),
    ("attachallover", FORCE),
    ("attachthis", FORCE),
    ("attachallthis", FORCE),
    ("attachthisover", FORCE),
    ("attachallthisover", FORCE),
    ("detach", FORCE),
    ("detachall", FORCE),
    ("detachthis", FORCE),
    ("detachallthis", FORCE),
    ("remattach", FORCE),
    ("remoutfit", FORCE),
    ("addoutfit", FORCE),
    ("addoutfitall", FORCE),
    ("addoutfitover", FORCE),
    ("addoutfitallover", FORCE),
    ("addoutfitthis", FORCE),
    ("addoutfitallthis", FORCE),
    ("addoutfitthisover", FORCE),
    ("addoutfitallthisover", FORCE),
    ("attachoverorreplace", FORCE),
    ("attachalloverorreplace", FORCE),
    ("attachthisoverorreplace", FORCE),
    ("attachallthisoverorreplace", FORCE),
    ("adjustheight", FORCE),
    ("detachme", FORCE),
    ("fly", FORCE),
    ("setcam_focus", FORCE | EXPERIMENTAL),
    ("setcam_eyeoffset", FORCE),
    ("setcam_eyeoffsetscale", FORCE),
    ("setcam_focusoffset", FORCE),
    ("setcam_fov", FORCE | EXPERIMENTAL),
    ("setcam_mode", FORCE | EXPERIMENTAL),
    ("setgroup", FORCE),
    ("sit", FORCE),
    ("sitground", FORCE),
    ("tpto", FORCE),
    ("unsit", FORCE),
    ("findfolder", REPLY),
    ("findfolders", REPLY),
    ("getaddattachnames", REPLY | EXPERIMENTAL),
    ("getaddoutfitnames", REPLY | EXPERIMENTAL),
    ("getattach", REPLY),
    ("getattachnames", REPLY | EXPERIMENTAL),
    ("getcam_avdist", REPLY | EXPERIMENTAL),
    ("getcam_avdistmin", REPLY | EXPERIMENTAL),
    ("getcam_avdistmax", REPLY | EXPERIMENTAL),
    ("getcam_fov", REPLY | EXPERIMENTAL),
    ("getcam_fovmin", REPLY | EXPERIMENTAL),
    ("getcam_fovmax", REPLY | EXPERIMENTAL),
    ("getcam_textures", REPLY | EXPERIMENTAL),
    ("getcommand", REPLY),
    ("getgroup", REPLY),
    ("getheightoffset", REPLY),
    ("getinv", REPLY),
    ("getinvworn", REPLY),
    ("getoutfit", REPLY),
    ("getoutfitnames", REPLY | EXPERIMENTAL),
    ("getpath", REPLY),
    ("getpathnew", REPLY),
    ("getremattachnames", REPLY | EXPERIMENTAL),
    ("getremoutfitnames", REPLY | EXPERIMENTAL),
    ("getsitid", REPLY),
    ("getstatus", REPLY),
    ("getstatusall", REPLY),
    ("version", REPLY),
    ("versionnew", REPLY),
    ("versionnum", REPLY),
];

/// RlvBehaviourDictionary::getCommands: every entry whose type matches `kind`
/// (0, RLV_TYPE_UNKNOWN, matches all), in order and with the duplicates of a
/// name registered for several types; a strict entry adds its "_sec" name
/// right after it. `filter` is a substring match, as `std::string::find`.
pub(super) fn commands(filter: &str, kind: u8, experimental: bool) -> Vec<String> {
    let mut list = Vec::new();
    for &(name, flags) in ENTRIES {
        if !experimental && flags & EXPERIMENTAL != 0 {
            continue;
        }
        if kind != 0 && flags & kind == 0 {
            continue;
        }
        if name.contains(filter) {
            list.push(name.to_owned());
        }
        if flags & STRICT != 0 {
            let strict = format!("{name}_sec");
            if strict.contains(filter) {
                list.push(strict);
            }
        }
    }
    list
}
