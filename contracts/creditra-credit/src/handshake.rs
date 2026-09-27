use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Deps, DepsMut, StdResult, Storage};
use cw_storage_plus::Item;

/// Protocol version for cross-contract handshake negotiation.
///
/// Uses a (major, minor) scheme where major version changes
/// indicate breaking protocol changes and minor version bumps
/// indicate backward-compatible additions.
#[cw_serde]
#[derive(Copy, Eq, Ord, PartialOrd)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ProtocolVersion {
    /// Check whether two versions share a common major version,
    /// meaning they *may* be wire-compatible.
    pub fn is_compatible_with(&self, other: &ProtocolVersion) -> bool {
        self.major == other.major
    }

    /// Check whether `self` meets or exceeds a minimum version floor.
    /// Major must match and minor must be >= the minimum.
    pub fn meets_minimum(&self, min: &ProtocolVersion) -> bool {
        self.major == min.major && self.minor >= min.minor
    }

    /// Negotiate a mutually supported protocol version between two peers.
    ///
    /// # Conditions
    ///
    /// 1. Both sides must share the same major version.
    /// 2. `our_version` must meet `their_min_compat` (we are recent enough for them).
    /// 3. `their_version` must meet `our_min_compat` (they are recent enough for us).
    ///
    /// When all conditions are satisfied the negotiated version is the
    /// *lower* of the two actual versions, ensuring both sides can
    /// communicate under that protocol revision.
    pub fn negotiate(
        our_version: &ProtocolVersion,
        their_version: &ProtocolVersion,
        our_min_compat: &ProtocolVersion,
        their_min_compat: &ProtocolVersion,
    ) -> Option<ProtocolVersion> {
        if our_version.major != their_version.major {
            return None;
        }
        if !our_version.meets_minimum(their_min_compat) {
            return None;
        }
        if !their_version.meets_minimum(our_min_compat) {
            return None;
        }
        Some(ProtocolVersion {
            major: our_version.major,
            minor: our_version.minor.min(their_version.minor),
        })
    }
}

/// The current protocol version of this contract.
pub const CURRENT_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

/// The minimum protocol version this contract can interoperate with.
pub const MIN_COMPATIBLE_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

/// Storage item for the on-chain protocol version.
pub const PROTOCOL_VERSION: Item<ProtocolVersion> = Item::new("protocol_version");

/// Initialize the stored protocol version on first deploy.
/// Idempotent – safe to call on re-instantiation.
pub fn initialize_version(storage: &mut dyn Storage) -> StdResult<()> {
    if PROTOCOL_VERSION.may_load(storage)?.is_none() {
        PROTOCOL_VERSION.save(storage, &CURRENT_PROTOCOL_VERSION)?;
    }
    Ok(())
}

/// Return the stored protocol version.
pub fn query_protocol_version(deps: Deps) -> StdResult<ProtocolVersion> {
    PROTOCOL_VERSION.load(deps.storage)
}

/// Overwrite the stored protocol version.  Use during upgrades.
pub fn set_protocol_version(deps: DepsMut, version: ProtocolVersion) -> StdResult<ProtocolVersion> {
    PROTOCOL_VERSION.save(deps.storage, &version)?;
    Ok(version)
}

/// Verify that two versions are mutually compatible.
///
/// Each version must be wire-compatible with the other
/// (same major version in both directions).
pub fn verify_peer_version(our_version: &ProtocolVersion, peer_version: &ProtocolVersion) -> bool {
    our_version.is_compatible_with(peer_version) && peer_version.is_compatible_with(our_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::mock_dependencies;

    fn v(major: u32, minor: u32) -> ProtocolVersion {
        ProtocolVersion { major, minor }
    }

    mod is_compatible_with {
        use super::*;

        #[test]
        fn same_major() {
            assert!(v(1, 0).is_compatible_with(&v(1, 5)));
            assert!(v(2, 3).is_compatible_with(&v(2, 0)));
        }

        #[test]
        fn different_major() {
            assert!(!v(1, 0).is_compatible_with(&v(2, 0)));
            assert!(!v(3, 1).is_compatible_with(&v(2, 9)));
        }

        #[test]
        fn equal_versions() {
            assert!(v(1, 0).is_compatible_with(&v(1, 0)));
        }
    }

    mod meets_minimum {
        use super::*;

        #[test]
        fn exactly_at_minimum() {
            assert!(v(1, 0).meets_minimum(&v(1, 0)));
        }

        #[test]
        fn above_minimum() {
            assert!(v(1, 5).meets_minimum(&v(1, 3)));
        }

        #[test]
        fn below_minimum() {
            assert!(!v(1, 2).meets_minimum(&v(1, 5)));
        }

        #[test]
        fn different_major_even_with_high_minor() {
            assert!(!v(2, 99).meets_minimum(&v(1, 0)));
        }
    }

    mod negotiate {
        use super::*;

        #[test]
        fn both_sides_meet_minimums() {
            let result = ProtocolVersion::negotiate(&v(1, 5), &v(1, 3), &v(1, 1), &v(1, 0));
            assert_eq!(result, Some(v(1, 3)));
        }

        #[test]
        fn incompatible_major_versions() {
            let result = ProtocolVersion::negotiate(&v(1, 0), &v(2, 0), &v(1, 0), &v(2, 0));
            assert_eq!(result, None);
        }

        #[test]
        fn our_version_too_low_for_their_minimum() {
            let result = ProtocolVersion::negotiate(&v(1, 0), &v(1, 5), &v(1, 0), &v(1, 2));
            assert_eq!(result, None);
        }

        #[test]
        fn their_version_too_low_for_our_minimum() {
            let result = ProtocolVersion::negotiate(&v(1, 5), &v(1, 0), &v(1, 3), &v(1, 0));
            assert_eq!(result, None);
        }

        #[test]
        fn picks_lower_of_two_versions() {
            let result = ProtocolVersion::negotiate(&v(1, 5), &v(1, 3), &v(1, 0), &v(1, 0));
            assert_eq!(result, Some(v(1, 3)));

            let result = ProtocolVersion::negotiate(&v(1, 2), &v(1, 7), &v(1, 0), &v(1, 0));
            assert_eq!(result, Some(v(1, 2)));
        }

        #[test]
        fn equal_versions_negotiate_to_self() {
            let result = ProtocolVersion::negotiate(&v(1, 0), &v(1, 0), &v(1, 0), &v(1, 0));
            assert_eq!(result, Some(v(1, 0)));
        }

        #[test]
        fn both_sides_exactly_at_each_others_minimum() {
            let result = ProtocolVersion::negotiate(&v(1, 2), &v(1, 3), &v(1, 1), &v(1, 0));
            assert_eq!(result, Some(v(1, 2)));
        }

        #[test]
        fn zero_version() {
            let result = ProtocolVersion::negotiate(&v(0, 0), &v(0, 0), &v(0, 0), &v(0, 0));
            assert_eq!(result, Some(v(0, 0)));
        }

        #[test]
        fn version_zero_and_one_different_major() {
            let result = ProtocolVersion::negotiate(&v(0, 5), &v(1, 0), &v(0, 0), &v(1, 0));
            assert_eq!(result, None);
        }
    }

    mod verify_peer_version {
        use super::*;

        #[test]
        fn same_major_is_compatible() {
            assert!(verify_peer_version(&v(1, 0), &v(1, 2)));
            assert!(verify_peer_version(&v(2, 5), &v(2, 0)));
        }

        #[test]
        fn different_major_is_incompatible() {
            assert!(!verify_peer_version(&v(1, 0), &v(2, 0)));
            assert!(!verify_peer_version(&v(3, 1), &v(2, 9)));
        }
    }

    mod storage {
        use super::*;

        #[test]
        fn initialize_version_sets_default() {
            let mut deps = mock_dependencies();
            initialize_version(deps.as_mut().storage).unwrap();
            let stored = PROTOCOL_VERSION.load(deps.as_ref().storage).unwrap();
            assert_eq!(stored, CURRENT_PROTOCOL_VERSION);
        }

        #[test]
        fn initialize_version_is_idempotent() {
            let mut deps = mock_dependencies();
            initialize_version(deps.as_mut().storage).unwrap();

            let custom = v(2, 0);
            PROTOCOL_VERSION
                .save(deps.as_mut().storage, &custom)
                .unwrap();

            initialize_version(deps.as_mut().storage).unwrap();
            let stored = PROTOCOL_VERSION.load(deps.as_ref().storage).unwrap();
            assert_eq!(stored, custom);
        }

        #[test]
        fn query_protocol_version_returns_stored() {
            let mut deps = mock_dependencies();
            initialize_version(deps.as_mut().storage).unwrap();

            let queried = query_protocol_version(deps.as_ref()).unwrap();
            assert_eq!(queried, CURRENT_PROTOCOL_VERSION);
        }

        #[test]
        fn set_protocol_version_overwrites() {
            let mut deps = mock_dependencies();
            initialize_version(deps.as_mut().storage).unwrap();

            let new_ver = v(2, 1);
            let returned = set_protocol_version(deps.as_mut(), new_ver).unwrap();
            assert_eq!(returned, new_ver);

            let stored = PROTOCOL_VERSION.load(deps.as_ref().storage).unwrap();
            assert_eq!(stored, new_ver);
        }

        #[test]
        fn set_protocol_version_returns_new_version() {
            let mut deps = mock_dependencies();
            let ver = v(3, 0);
            let result = set_protocol_version(deps.as_mut(), ver).unwrap();
            assert_eq!(result, ver);
        }
    }
}
