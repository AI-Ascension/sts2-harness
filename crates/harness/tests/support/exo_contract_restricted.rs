// SPDX-License-Identifier: MIT

use super::*;
use serde_json::json;

fn valid_trusted() -> ExoTrustedConfiguration {
    ExoTrustedConfiguration {
        identity: complete_identity(),
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: restricted_profile(),
    }
}

#[test]
fn reviewed_restricted_profile_passes_with_an_empty_catalog() {
    let profile = restricted_profile();
    profile
        .validate()
        .expect("reviewed private profile is valid");
    assert!(REVIEWED_MODEL_TOOLS.is_empty());
    assert!(profile.tool_catalog.tools.is_empty());
    assert_eq!(ExoToolCatalog::reviewed().validate(), Ok(()));
    assert_eq!(profile.tool_catalog.catalog_digest().len(), 64);
    assert_eq!(profile.profile_digest().len(), 64);
    assert_eq!(profile.profile_digest(), profile.profile_digest());
}

#[test]
fn tool_catalog_rejects_every_unreviewed_shape() {
    let cases = [
        (vec![String::new()], ExoToolCatalogError::EmptyToolName),
        (
            vec!["read file".to_owned()],
            ExoToolCatalogError::InvalidToolName,
        ),
        (
            vec!["shell".to_owned(), "shell".to_owned()],
            ExoToolCatalogError::DuplicateTool,
        ),
        (vec!["shell".to_owned()], ExoToolCatalogError::UnknownTool),
        (
            vec!["read_only_query".to_owned()],
            ExoToolCatalogError::UnknownTool,
        ),
    ];
    for (tools, expected) in cases {
        let catalog = ExoToolCatalog { tools };
        assert_eq!(catalog.validate(), Err(expected));
    }
}

#[test]
fn private_state_policy_rejects_unsafe_roots() {
    let valid = restricted_profile().state;
    let cases = [
        (
            ExoPrivateStatePolicy {
                state_root: "relative/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::PathNotAbsolute(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/var/lib/../etc".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::PathEscapes(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/home/analyst/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/root/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/Users/analyst/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/var/home/analyst/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/etc/sts2/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/srv/sts2/SlayTheSpire2-1.2.3/state".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                state_root: "/srv/SteamLibrary/steamapps/common/SlayTheSpire2/saves".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::State),
        ),
        (
            ExoPrivateStatePolicy {
                temp_root: "/".to_owned(),
                ..valid.clone()
            },
            ExoPrivateStateError::ForbiddenPath(PrivateRootKind::Temp),
        ),
        (
            ExoPrivateStatePolicy {
                cache_root: format!("{}/nested", valid.state_root),
                ..valid.clone()
            },
            ExoPrivateStateError::NestedRoot,
        ),
        (
            ExoPrivateStatePolicy {
                cache_root: valid.state_root.clone(),
                ..valid.clone()
            },
            ExoPrivateStateError::DuplicateRoot,
        ),
    ];
    for (policy, expected) in cases {
        assert_eq!(policy.validate(), Err(expected));
    }
}

#[test]
fn private_state_policy_bounds_quota_retention_and_permissions() {
    let valid = restricted_profile().state;
    let cases = [
        (
            ExoPrivateStatePolicy {
                quota_bytes: 0,
                ..valid.clone()
            },
            ExoPrivateStateError::InvalidQuota,
        ),
        (
            ExoPrivateStatePolicy {
                quota_bytes: EXO_RESTRICTED_MAX_QUOTA_BYTES + 1,
                ..valid.clone()
            },
            ExoPrivateStateError::InvalidQuota,
        ),
        (
            ExoPrivateStatePolicy {
                max_retention_days: 0,
                ..valid.clone()
            },
            ExoPrivateStateError::InvalidRetention,
        ),
        (
            ExoPrivateStatePolicy {
                max_retention_days: EXO_RESTRICTED_MAX_RETENTION_DAYS + 1,
                ..valid.clone()
            },
            ExoPrivateStateError::InvalidRetention,
        ),
        (
            ExoPrivateStatePolicy {
                permissions_octal: 0o755,
                ..valid.clone()
            },
            ExoPrivateStateError::UnsafePermissions,
        ),
        (
            ExoPrivateStatePolicy {
                permissions_octal: 0o777,
                ..valid.clone()
            },
            ExoPrivateStateError::UnsafePermissions,
        ),
    ];
    for (policy, expected) in cases {
        assert_eq!(policy.validate(), Err(expected));
    }
    assert_eq!(valid.validate(), Ok(()));
}

#[test]
fn changed_catalog_and_profile_digests_are_detectable() {
    let reviewed = ExoToolCatalog::reviewed();
    let changed_catalog = ExoToolCatalog {
        tools: vec!["shell".to_owned()],
    };
    assert_ne!(reviewed.catalog_digest(), changed_catalog.catalog_digest());

    let profile = restricted_profile();
    let mut changed_profile = profile.clone();
    changed_profile
        .tool_catalog
        .tools
        .push("read_only_query".to_owned());
    assert_ne!(profile.profile_digest(), changed_profile.profile_digest());

    let mut changed_state = profile.clone();
    changed_state.state.quota_bytes = EXO_RESTRICTED_MAX_QUOTA_BYTES;
    assert_ne!(profile.profile_digest(), changed_state.profile_digest());
}

#[test]
fn policy_digest_binds_every_field_and_domain_separates_values() {
    let reviewed = restricted_profile().state;
    let original = reviewed.policy_digest();
    let mutations = [
        ExoPrivateStatePolicy {
            state_root: String::from("/var/lib/sts2-harness/other-state"),
            ..reviewed.clone()
        },
        ExoPrivateStatePolicy {
            cache_root: String::from("/var/lib/sts2-harness/other-cache"),
            ..reviewed.clone()
        },
        ExoPrivateStatePolicy {
            temp_root: String::from("/var/lib/sts2-harness/other-temp"),
            ..reviewed.clone()
        },
        ExoPrivateStatePolicy {
            quota_bytes: reviewed.quota_bytes + 1,
            ..reviewed.clone()
        },
        ExoPrivateStatePolicy {
            max_retention_days: reviewed.max_retention_days + 1,
            ..reviewed.clone()
        },
        ExoPrivateStatePolicy {
            permissions_octal: reviewed.permissions_octal ^ 1,
            ..reviewed.clone()
        },
    ];
    for (index, mutation) in mutations.iter().enumerate() {
        assert_ne!(
            original,
            mutation.policy_digest(),
            "policy_digest must bind field index {index}"
        );
    }

    // Values that would concatenate identically without a separator/length prefix stay distinct.
    let left = ExoPrivateStatePolicy {
        state_root: String::from("ab"),
        cache_root: String::from("c"),
        ..reviewed.clone()
    };
    let right = ExoPrivateStatePolicy {
        state_root: String::from("a"),
        cache_root: String::from("bc"),
        ..reviewed.clone()
    };
    assert_ne!(left.policy_digest(), right.policy_digest());

    // A Unicode field value contributes distinctly.
    let mut unicode = reviewed;
    unicode.state_root = String::from("/var/lib/sts2-harness/st\u{e9}te");
    assert_ne!(original, unicode.policy_digest());
}

#[test]
fn restricted_types_are_closed_and_required() {
    let mut catalog_value =
        serde_json::to_value(ExoToolCatalog::reviewed()).expect("catalog serializes");
    catalog_value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ExoToolCatalog>(catalog_value).is_err());

    let mut policy_value =
        serde_json::to_value(restricted_profile().state).expect("policy serializes");
    policy_value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ExoPrivateStatePolicy>(policy_value).is_err());

    let mut profile_value = serde_json::to_value(restricted_profile()).expect("profile serializes");
    profile_value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<sts2_harness::ExoRestrictedProfile>(profile_value).is_err());

    let mut trusted_value =
        serde_json::to_value(valid_trusted()).expect("trusted config serializes");
    let removed = trusted_value
        .as_object_mut()
        .and_then(|object| object.remove("restricted"));
    assert!(
        removed.is_some(),
        "trusted config carries a restricted field"
    );
    assert!(serde_json::from_value::<ExoTrustedConfiguration>(trusted_value).is_err());
}

#[test]
fn preflight_fails_closed_when_the_restricted_profile_is_invalid() {
    let mut descriptor = ExoCapabilityDescriptor::source_review().expect("descriptor");
    enable_minimum_capabilities(&mut descriptor);
    let trusted = valid_trusted();
    assert!(preflight_with_identity(descriptor.clone(), &trusted).is_ok());

    let mut bad_tools = trusted.clone();
    bad_tools
        .restricted
        .tool_catalog
        .tools
        .push("shell".to_owned());
    assert_eq!(
        preflight_with_identity(descriptor.clone(), &bad_tools),
        Err(ExoPreflightError::InvalidRestrictedProfile(
            ExoRestrictedError::ToolCatalog(ExoToolCatalogError::UnknownTool)
        ))
    );

    let mut bad_state = trusted;
    bad_state.restricted.state.cache_root = "/home/analyst/cache".to_owned();
    assert_eq!(
        preflight_with_identity(descriptor, &bad_state),
        Err(ExoPreflightError::InvalidRestrictedProfile(
            ExoRestrictedError::PrivateState(ExoPrivateStateError::ForbiddenPath(
                PrivateRootKind::Cache
            ))
        ))
    );
}

#[test]
fn synthetic_forbidden_names_and_paths_are_rejected_without_leaking() {
    for sentinel in [
        "shell",
        "read_file",
        "install_tool",
        "browser",
        "SlayTheSpire2",
    ] {
        let catalog = ExoToolCatalog {
            tools: vec![sentinel.to_owned()],
        };
        let error = catalog.validate().expect_err("sentinel tool is rejected");
        assert!(!format!("{error}").contains(sentinel));
    }

    let path_sentinel = "/home/analyst/.steam/steamapps/common/SlayTheSpire2/saves";
    let policy = ExoPrivateStatePolicy {
        state_root: path_sentinel.to_owned(),
        ..restricted_profile().state
    };
    let error = policy.validate().expect_err("sentinel path is rejected");
    let rendered = format!("{error}");
    assert!(!rendered.contains(path_sentinel));
    assert!(!rendered.contains("analyst"));
    assert!(!rendered.contains("Steam"));
    assert!(!rendered.contains("Slay"));
}
