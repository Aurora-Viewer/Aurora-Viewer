//! Firestorm's catalogue of group abilities (indra/newview/skins/default/xui/
//! en/role_actions.xml, read by LLGroupMgr::parseRoleActions, originally
//! LGPL 2.1), generated: the sets in their order, each ability with its
//! power bit (roles_constants.h) and its texts (`tr!` keys).

/// One ability: its power bit, short and long texts.
pub struct Ability {
    pub bit: u32,
    pub name: &'static str,
    pub help: &'static str,
}

/// A set of abilities, its icon and its texts.
pub struct AbilitySet {
    pub icon: &'static str,
    pub name: &'static str,
    pub help: &'static str,
    pub abilities: Vec<Ability>,
}

pub fn catalogue() -> Vec<AbilitySet> {
    vec![
        AbilitySet {
            icon: "users",
            name: aurora_i18n::tr!("group-abilities-membership"),
            help: aurora_i18n::tr!("group-abilities-membership-help"),
            abilities: vec![
                Ability {
                    bit: 1,
                    name: aurora_i18n::tr!("group-ability-member-invite"),
                    help: aurora_i18n::tr!("group-ability-member-invite-help"),
                },
                Ability {
                    bit: 2,
                    name: aurora_i18n::tr!("group-ability-member-eject"),
                    help: aurora_i18n::tr!("group-ability-member-eject-help"),
                },
                Ability {
                    bit: 51,
                    name: aurora_i18n::tr!("group-ability-allow-ban"),
                    help: aurora_i18n::tr!("group-ability-allow-ban-help"),
                },
                Ability {
                    bit: 3,
                    name: aurora_i18n::tr!("group-ability-member-options"),
                    help: aurora_i18n::tr!("group-ability-member-options-help"),
                },
            ],
        },
        AbilitySet {
            icon: "crown",
            name: aurora_i18n::tr!("group-abilities-roles"),
            help: aurora_i18n::tr!("group-abilities-roles-help"),
            abilities: vec![
                Ability {
                    bit: 4,
                    name: aurora_i18n::tr!("group-ability-role-create"),
                    help: aurora_i18n::tr!("group-ability-role-create-help"),
                },
                Ability {
                    bit: 5,
                    name: aurora_i18n::tr!("group-ability-role-delete"),
                    help: aurora_i18n::tr!("group-ability-role-delete-help"),
                },
                Ability {
                    bit: 6,
                    name: aurora_i18n::tr!("group-ability-role-properties"),
                    help: aurora_i18n::tr!("group-ability-role-properties-help"),
                },
                Ability {
                    bit: 7,
                    name: aurora_i18n::tr!("group-ability-role-assign-member-limited"),
                    help: aurora_i18n::tr!("group-ability-role-assign-member-limited-help"),
                },
                Ability {
                    bit: 8,
                    name: aurora_i18n::tr!("group-ability-role-assign-member"),
                    help: aurora_i18n::tr!("group-ability-role-assign-member-help"),
                },
                Ability {
                    bit: 9,
                    name: aurora_i18n::tr!("group-ability-role-remove-member"),
                    help: aurora_i18n::tr!("group-ability-role-remove-member-help"),
                },
                Ability {
                    bit: 10,
                    name: aurora_i18n::tr!("group-ability-role-change-actions"),
                    help: aurora_i18n::tr!("group-ability-role-change-actions-help"),
                },
            ],
        },
        AbilitySet {
            icon: "id-filled",
            name: aurora_i18n::tr!("group-abilities-group-identity"),
            help: aurora_i18n::tr!("group-abilities-group-identity-help"),
            abilities: vec![Ability {
                bit: 11,
                name: aurora_i18n::tr!("group-ability-group-change-identity"),
                help: aurora_i18n::tr!("group-ability-group-change-identity-help"),
            }],
        },
        AbilitySet {
            icon: "map-2",
            name: aurora_i18n::tr!("group-abilities-parcel-management"),
            help: aurora_i18n::tr!("group-abilities-parcel-management-help"),
            abilities: vec![
                Ability {
                    bit: 12,
                    name: aurora_i18n::tr!("group-ability-land-deed"),
                    help: aurora_i18n::tr!("group-ability-land-deed-help"),
                },
                Ability {
                    bit: 13,
                    name: aurora_i18n::tr!("group-ability-land-release"),
                    help: aurora_i18n::tr!("group-ability-land-release-help"),
                },
                Ability {
                    bit: 14,
                    name: aurora_i18n::tr!("group-ability-land-set-sale-info"),
                    help: aurora_i18n::tr!("group-ability-land-set-sale-info-help"),
                },
                Ability {
                    bit: 15,
                    name: aurora_i18n::tr!("group-ability-land-divide-join"),
                    help: aurora_i18n::tr!("group-ability-land-divide-join-help"),
                },
            ],
        },
        AbilitySet {
            icon: "map-pin-filled",
            name: aurora_i18n::tr!("group-abilities-parcel-identity"),
            help: aurora_i18n::tr!("group-abilities-parcel-identity-help"),
            abilities: vec![
                Ability {
                    bit: 17,
                    name: aurora_i18n::tr!("group-ability-land-find-places"),
                    help: aurora_i18n::tr!("group-ability-land-find-places-help"),
                },
                Ability {
                    bit: 18,
                    name: aurora_i18n::tr!("group-ability-land-change-identity"),
                    help: aurora_i18n::tr!("group-ability-land-change-identity-help"),
                },
                Ability {
                    bit: 19,
                    name: aurora_i18n::tr!("group-ability-land-set-landing-point"),
                    help: aurora_i18n::tr!("group-ability-land-set-landing-point-help"),
                },
            ],
        },
        AbilitySet {
            icon: "adjustments-filled",
            name: aurora_i18n::tr!("group-abilities-parcel-settings"),
            help: aurora_i18n::tr!("group-abilities-parcel-settings-help"),
            abilities: vec![
                Ability {
                    bit: 20,
                    name: aurora_i18n::tr!("group-ability-land-change-media"),
                    help: aurora_i18n::tr!("group-ability-land-change-media-help"),
                },
                Ability {
                    bit: 21,
                    name: aurora_i18n::tr!("group-ability-land-edit"),
                    help: aurora_i18n::tr!("group-ability-land-edit-help"),
                },
                Ability {
                    bit: 22,
                    name: aurora_i18n::tr!("group-ability-land-options"),
                    help: aurora_i18n::tr!("group-ability-land-options-help"),
                },
                Ability {
                    bit: 46,
                    name: aurora_i18n::tr!("group-ability-land-change-environment"),
                    help: aurora_i18n::tr!("group-ability-land-change-environment-help"),
                },
            ],
        },
        AbilitySet {
            icon: "bolt-filled",
            name: aurora_i18n::tr!("group-abilities-parcel-powers"),
            help: aurora_i18n::tr!("group-abilities-parcel-powers-help"),
            abilities: vec![
                Ability {
                    bit: 23,
                    name: aurora_i18n::tr!("group-ability-land-allow-edit-land"),
                    help: aurora_i18n::tr!("group-ability-land-allow-edit-land-help"),
                },
                Ability {
                    bit: 24,
                    name: aurora_i18n::tr!("group-ability-land-allow-fly"),
                    help: aurora_i18n::tr!("group-ability-land-allow-fly-help"),
                },
                Ability {
                    bit: 25,
                    name: aurora_i18n::tr!("group-ability-land-allow-create"),
                    help: aurora_i18n::tr!("group-ability-land-allow-create-help"),
                },
                Ability {
                    bit: 26,
                    name: aurora_i18n::tr!("group-ability-land-allow-direct-teleport"),
                    help: aurora_i18n::tr!("group-ability-land-allow-direct-teleport-help"),
                },
                Ability {
                    bit: 28,
                    name: aurora_i18n::tr!("group-ability-land-allow-set-home"),
                    help: aurora_i18n::tr!("group-ability-land-allow-set-home-help"),
                },
                Ability {
                    bit: 41,
                    name: aurora_i18n::tr!("group-ability-land-allow-host-event"),
                    help: aurora_i18n::tr!("group-ability-land-allow-host-event-help"),
                },
            ],
        },
        AbilitySet {
            icon: "lock-filled",
            name: aurora_i18n::tr!("group-abilities-parcel-access"),
            help: aurora_i18n::tr!("group-abilities-parcel-access-help"),
            abilities: vec![
                Ability {
                    bit: 29,
                    name: aurora_i18n::tr!("group-ability-land-manage-allowed"),
                    help: aurora_i18n::tr!("group-ability-land-manage-allowed-help"),
                },
                Ability {
                    bit: 30,
                    name: aurora_i18n::tr!("group-ability-land-manage-banned"),
                    help: aurora_i18n::tr!("group-ability-land-manage-banned-help"),
                },
                Ability {
                    bit: 31,
                    name: aurora_i18n::tr!("group-ability-land-manage-passes"),
                    help: aurora_i18n::tr!("group-ability-land-manage-passes-help"),
                },
                Ability {
                    bit: 32,
                    name: aurora_i18n::tr!("group-ability-land-admin"),
                    help: aurora_i18n::tr!("group-ability-land-admin-help"),
                },
            ],
        },
        AbilitySet {
            icon: "photo-filled",
            name: aurora_i18n::tr!("group-abilities-parcel-content"),
            help: aurora_i18n::tr!("group-abilities-parcel-content-help"),
            abilities: vec![
                Ability {
                    bit: 48,
                    name: aurora_i18n::tr!("group-ability-land-return-group-owned"),
                    help: aurora_i18n::tr!("group-ability-land-return-group-owned-help"),
                },
                Ability {
                    bit: 33,
                    name: aurora_i18n::tr!("group-ability-land-return-group-set"),
                    help: aurora_i18n::tr!("group-ability-land-return-group-set-help"),
                },
                Ability {
                    bit: 34,
                    name: aurora_i18n::tr!("group-ability-land-return-non-group"),
                    help: aurora_i18n::tr!("group-ability-land-return-non-group-help"),
                },
                Ability {
                    bit: 35,
                    name: aurora_i18n::tr!("group-ability-land-gardening"),
                    help: aurora_i18n::tr!("group-ability-land-gardening-help"),
                },
            ],
        },
        AbilitySet {
            icon: "package",
            name: aurora_i18n::tr!("group-abilities-object-management"),
            help: aurora_i18n::tr!("group-abilities-object-management-help"),
            abilities: vec![
                Ability {
                    bit: 36,
                    name: aurora_i18n::tr!("group-ability-object-deed"),
                    help: aurora_i18n::tr!("group-ability-object-deed-help"),
                },
                Ability {
                    bit: 38,
                    name: aurora_i18n::tr!("group-ability-object-manipulate"),
                    help: aurora_i18n::tr!("group-ability-object-manipulate-help"),
                },
                Ability {
                    bit: 39,
                    name: aurora_i18n::tr!("group-ability-object-set-sale"),
                    help: aurora_i18n::tr!("group-ability-object-set-sale-help"),
                },
            ],
        },
        AbilitySet {
            icon: "coin-filled",
            name: aurora_i18n::tr!("group-abilities-accounting"),
            help: aurora_i18n::tr!("group-abilities-accounting-help"),
            abilities: vec![Ability {
                bit: 40,
                name: aurora_i18n::tr!("group-ability-accounting-accountable"),
                help: aurora_i18n::tr!("group-ability-accounting-accountable-help"),
            }],
        },
        AbilitySet {
            icon: "speakerphone",
            name: aurora_i18n::tr!("group-abilities-notices"),
            help: aurora_i18n::tr!("group-abilities-notices-help"),
            abilities: vec![
                Ability {
                    bit: 42,
                    name: aurora_i18n::tr!("group-ability-notices-send"),
                    help: aurora_i18n::tr!("group-ability-notices-send-help"),
                },
                Ability {
                    bit: 43,
                    name: aurora_i18n::tr!("group-ability-notices-receive"),
                    help: aurora_i18n::tr!("group-ability-notices-receive-help"),
                },
            ],
        },
        AbilitySet {
            icon: "messages-filled",
            name: aurora_i18n::tr!("group-abilities-chat"),
            help: aurora_i18n::tr!("group-abilities-chat-help"),
            abilities: vec![
                Ability {
                    bit: 16,
                    name: aurora_i18n::tr!("group-ability-join-group-chat"),
                    help: aurora_i18n::tr!("group-ability-join-group-chat-help"),
                },
                Ability {
                    bit: 27,
                    name: aurora_i18n::tr!("group-ability-join-voice-chat"),
                    help: aurora_i18n::tr!("group-ability-join-voice-chat-help"),
                },
                Ability {
                    bit: 37,
                    name: aurora_i18n::tr!("group-ability-moderate-group-chat"),
                    help: aurora_i18n::tr!("group-ability-moderate-group-chat-help"),
                },
            ],
        },
        AbilitySet {
            icon: "key-filled",
            name: aurora_i18n::tr!("group-abilities-experience-tools-experience"),
            help: aurora_i18n::tr!("group-abilities-experience-tools-experience-help"),
            abilities: vec![
                Ability {
                    bit: 49,
                    name: aurora_i18n::tr!("group-ability-experience-admin"),
                    help: aurora_i18n::tr!("group-ability-experience-admin-help"),
                },
                Ability {
                    bit: 50,
                    name: aurora_i18n::tr!("group-ability-experience-contributor"),
                    help: aurora_i18n::tr!("group-ability-experience-contributor-help"),
                },
            ],
        },
    ]
}
