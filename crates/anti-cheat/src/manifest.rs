use std::collections::BTreeMap;
use std::fmt;

/// Version of the manifest layout itself. Bump when field semantics change.
pub const MANIFEST_VERSION: u32 = 1;

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Deterministic, endian-independent 64-bit content hash (FNV-1a).
///
/// Chosen over a cryptographic digest deliberately: manifests are an
/// *accident and casual-tamper* boundary, not an integrity guarantee against a
/// determined attacker. A client that controls its own process can always
/// report whatever manifest it likes. Real binary integrity is EAC's job; see
/// `docs/SECURITY.md`.
pub fn content_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Mix a 64-bit value into a running hash, preserving order sensitivity.
fn mix_u64(hash: u64, value: u64) -> u64 {
    content_hash_bytes(&[hash.to_le_bytes(), value.to_le_bytes()].concat())
}

/// Hash of the set of loaded content packs.
///
/// Entries are held in a `BTreeMap` so the rolled-up hash is insertion-order
/// independent and identical on every machine.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct ContentManifest {
    entries: BTreeMap<String, u64>,
}

impl ContentManifest {
    pub fn new() -> Self {
        ContentManifest::default()
    }

    /// Register a content pack by name and precomputed hash.
    pub fn insert(&mut self, name: impl Into<String>, hash: u64) {
        self.entries.insert(name.into(), hash);
    }

    /// Register a content pack, hashing its raw bytes.
    pub fn insert_bytes(&mut self, name: impl Into<String>, bytes: &[u8]) {
        let hash = content_hash_bytes(bytes);
        self.entries.insert(name.into(), hash);
    }

    pub fn get(&self, name: &str) -> Option<u64> {
        self.entries.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &u64)> {
        self.entries.iter()
    }

    /// Rolled-up deterministic hash of every registered content pack.
    pub fn content_hash(&self) -> u64 {
        let mut hash = FNV_OFFSET_BASIS;
        for (name, entry_hash) in &self.entries {
            hash = mix_u64(hash, content_hash_bytes(name.as_bytes()));
            hash = mix_u64(hash, *entry_hash);
        }
        hash
    }
}

/// Build / protocol / content identity advertised by a client or server.
///
/// `protocol_version` mirrors `game_protocol::version::PROTOCOL_VERSION`. It is
/// carried as a plain `u32` rather than importing the protocol crate so that the
/// dependency edge stays `game-protocol -> anti-cheat` and never the reverse.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BuildManifest {
    /// Opaque build identifier, e.g. a CI build number or commit hash.
    pub build_id: String,
    /// Wire protocol version this build speaks.
    pub protocol_version: u32,
    /// Rolled-up hash of the loaded content packs.
    pub content_hash: u64,
    /// Whether this build is an unmodified official build.
    pub official: bool,
}

impl BuildManifest {
    /// A modded / private build manifest.
    pub fn new(build_id: impl Into<String>, protocol_version: u32, content_hash: u64) -> Self {
        BuildManifest {
            build_id: build_id.into(),
            protocol_version,
            content_hash,
            official: false,
        }
    }

    /// An unmodified official build manifest.
    pub fn official(build_id: impl Into<String>, protocol_version: u32, content_hash: u64) -> Self {
        BuildManifest {
            official: true,
            ..BuildManifest::new(build_id, protocol_version, content_hash)
        }
    }

    /// Build a manifest from a live content manifest.
    pub fn from_content(
        build_id: impl Into<String>,
        protocol_version: u32,
        content: &ContentManifest,
    ) -> Self {
        BuildManifest::new(build_id, protocol_version, content.content_hash())
    }

    /// Deterministic hash over every field, used for exact-match policies.
    pub fn manifest_hash(&self) -> u64 {
        let mut hash = FNV_OFFSET_BASIS;
        hash = mix_u64(hash, MANIFEST_VERSION as u64);
        hash = mix_u64(hash, content_hash_bytes(self.build_id.as_bytes()));
        hash = mix_u64(hash, self.protocol_version as u64);
        hash = mix_u64(hash, self.content_hash);
        hash = mix_u64(hash, u64::from(self.official));
        hash
    }
}

impl fmt::Display for BuildManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "build={} protocol={} content={:#018x} official={}",
            self.build_id, self.protocol_version, self.content_hash, self.official
        )
    }
}

/// Why a client manifest was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ManifestError {
    ProtocolVersionMismatch {
        expected: u32,
        actual: u32,
    },
    BuildIdMismatch {
        expected: String,
        actual: String,
    },
    ContentHashMismatch {
        expected: u64,
        actual: u64,
    },
    /// A modified client tried to join an official server.
    ModdedClientOnOfficialServer,
    /// A private server pinned a manifest hash and the client does not match it.
    UnapprovedCustomManifest {
        expected: u64,
        actual: u64,
    },
}

impl ManifestError {
    /// Manifest hash the server expected, where one applies.
    pub fn expected_hash(&self) -> u64 {
        match self {
            ManifestError::ContentHashMismatch { expected, .. } => *expected,
            ManifestError::UnapprovedCustomManifest { expected, .. } => *expected,
            _ => 0,
        }
    }

    /// Manifest hash the client reported, where one applies.
    pub fn actual_hash(&self) -> u64 {
        match self {
            ManifestError::ContentHashMismatch { actual, .. } => *actual,
            ManifestError::UnapprovedCustomManifest { actual, .. } => *actual,
            _ => 0,
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::ProtocolVersionMismatch { expected, actual } => write!(
                f,
                "Manifest protocol version mismatch: server {expected}, client {actual}"
            ),
            ManifestError::BuildIdMismatch { expected, actual } => write!(
                f,
                "Manifest build id mismatch: server '{expected}', client '{actual}'"
            ),
            ManifestError::ContentHashMismatch { expected, actual } => write!(
                f,
                "Manifest content hash mismatch: server {expected:#018x}, client {actual:#018x}"
            ),
            ManifestError::ModdedClientOnOfficialServer => write!(
                f,
                "Official servers only accept unmodified official client builds"
            ),
            ManifestError::UnapprovedCustomManifest { expected, actual } => write!(
                f,
                "Custom manifest not approved by this server: expected {expected:#018x}, got {actual:#018x}"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}

/// How strictly a server enforces build/protocol/content identity.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum ServerPolicy {
    /// Local development and single-player. No manifest enforcement at all.
    #[default]
    LocalDev,
    /// Private / co-op server. Protocol must match; content may be modded.
    ///
    /// A host that wants a specific mod set pins its manifest hash in
    /// `accepted_manifest_hash`, and every client must present exactly that.
    PrivateCustom { accepted_manifest_hash: Option<u64> },
    /// Official server. Client manifest must match the server's exactly and the
    /// client must be an unmodified official build.
    Official,
}

impl ServerPolicy {
    /// Whether this policy requires a client manifest before commands are honoured.
    pub fn requires_manifest(&self) -> bool {
        matches!(self, ServerPolicy::Official)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ServerPolicy::LocalDev => "local-dev",
            ServerPolicy::PrivateCustom { .. } => "private",
            ServerPolicy::Official => "official",
        }
    }

    /// Validate a client manifest against the server's own manifest.
    pub fn validate(
        &self,
        server: &BuildManifest,
        client: &BuildManifest,
    ) -> Result<(), ManifestError> {
        match self {
            ServerPolicy::LocalDev => Ok(()),
            ServerPolicy::PrivateCustom {
                accepted_manifest_hash,
            } => {
                if client.protocol_version != server.protocol_version {
                    return Err(ManifestError::ProtocolVersionMismatch {
                        expected: server.protocol_version,
                        actual: client.protocol_version,
                    });
                }
                match accepted_manifest_hash {
                    None => Ok(()),
                    Some(expected) if *expected == client.manifest_hash() => Ok(()),
                    Some(expected) => Err(ManifestError::UnapprovedCustomManifest {
                        expected: *expected,
                        actual: client.manifest_hash(),
                    }),
                }
            }
            ServerPolicy::Official => {
                if client.protocol_version != server.protocol_version {
                    return Err(ManifestError::ProtocolVersionMismatch {
                        expected: server.protocol_version,
                        actual: client.protocol_version,
                    });
                }
                if !client.official {
                    return Err(ManifestError::ModdedClientOnOfficialServer);
                }
                if client.build_id != server.build_id {
                    return Err(ManifestError::BuildIdMismatch {
                        expected: server.build_id.clone(),
                        actual: client.build_id.clone(),
                    });
                }
                if client.content_hash != server.content_hash {
                    return Err(ManifestError::ContentHashMismatch {
                        expected: server.content_hash,
                        actual: client.content_hash,
                    });
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_manifest() -> BuildManifest {
        BuildManifest::official("rts-engine-0.1.0+ci42", 1, 0xdead_beef)
    }

    #[test]
    fn test_content_hash_is_deterministic_and_order_independent() {
        let mut a = ContentManifest::new();
        a.insert_bytes("core", b"core-data");
        a.insert_bytes("walls", b"wall-data");

        let mut b = ContentManifest::new();
        b.insert_bytes("walls", b"wall-data");
        b.insert_bytes("core", b"core-data");

        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.content_hash(), a.content_hash());
    }

    #[test]
    fn test_content_hash_changes_when_content_changes() {
        let mut a = ContentManifest::new();
        a.insert_bytes("core", b"core-data");
        let base = a.content_hash();
        a.insert_bytes("core", b"core-data-modded");
        assert_ne!(base, a.content_hash());
    }

    #[test]
    fn test_manifest_hash_covers_every_field() {
        let base = server_manifest();
        let mut other = base.clone();
        other.official = false;
        assert_ne!(base.manifest_hash(), other.manifest_hash());
        let mut other = base.clone();
        other.protocol_version = 2;
        assert_ne!(base.manifest_hash(), other.manifest_hash());
        let mut other = base.clone();
        other.content_hash = 1;
        assert_ne!(base.manifest_hash(), other.manifest_hash());
        let mut other = base.clone();
        other.build_id = "other".to_string();
        assert_ne!(base.manifest_hash(), other.manifest_hash());
    }

    #[test]
    fn test_local_dev_policy_accepts_any_manifest() {
        let policy = ServerPolicy::LocalDev;
        let server = server_manifest();
        let modded = BuildManifest::new("modded-build", 999, 42);
        assert!(policy.validate(&server, &modded).is_ok());
        assert!(!policy.requires_manifest());
    }

    #[test]
    fn test_official_policy_requires_exact_manifest_match() {
        let policy = ServerPolicy::Official;
        let server = server_manifest();
        assert!(policy.validate(&server, &server.clone()).is_ok());
        assert!(policy.requires_manifest());

        let mut wrong_content = server.clone();
        wrong_content.content_hash = 7;
        assert_eq!(
            policy.validate(&server, &wrong_content),
            Err(ManifestError::ContentHashMismatch {
                expected: 0xdead_beef,
                actual: 7
            })
        );

        let mut wrong_build = server.clone();
        wrong_build.build_id = "rts-engine-0.1.0+ci41".to_string();
        assert!(matches!(
            policy.validate(&server, &wrong_build),
            Err(ManifestError::BuildIdMismatch { .. })
        ));
    }

    #[test]
    fn test_official_policy_rejects_modded_client() {
        let policy = ServerPolicy::Official;
        let server = server_manifest();
        let mut modded = server.clone();
        modded.official = false;
        assert_eq!(
            policy.validate(&server, &modded),
            Err(ManifestError::ModdedClientOnOfficialServer)
        );
    }

    #[test]
    fn test_private_server_opts_into_custom_manifest() {
        let server = server_manifest();
        let modded = BuildManifest::new("community-build", 1, 0xabc);

        // Unpinned private server accepts any content as long as protocol matches.
        let open = ServerPolicy::PrivateCustom {
            accepted_manifest_hash: None,
        };
        assert!(open.validate(&server, &modded).is_ok());

        // Pinned private server accepts exactly the approved custom manifest.
        let pinned = ServerPolicy::PrivateCustom {
            accepted_manifest_hash: Some(modded.manifest_hash()),
        };
        assert!(pinned.validate(&server, &modded).is_ok());

        let other = BuildManifest::new("other-community-build", 1, 0xabc);
        assert!(matches!(
            pinned.validate(&server, &other),
            Err(ManifestError::UnapprovedCustomManifest { .. })
        ));
    }

    #[test]
    fn test_protocol_version_mismatch_is_rejected_by_non_dev_policies() {
        let server = server_manifest();
        let stale = BuildManifest::official("rts-engine-0.1.0+ci42", 0, 0xdead_beef);
        for policy in [
            ServerPolicy::Official,
            ServerPolicy::PrivateCustom {
                accepted_manifest_hash: None,
            },
        ] {
            assert_eq!(
                policy.validate(&server, &stale),
                Err(ManifestError::ProtocolVersionMismatch {
                    expected: 1,
                    actual: 0
                })
            );
        }
    }

    #[test]
    fn test_manifest_error_exposes_hashes_for_telemetry() {
        let err = ManifestError::ContentHashMismatch {
            expected: 9,
            actual: 3,
        };
        assert_eq!(err.expected_hash(), 9);
        assert_eq!(err.actual_hash(), 3);
    }
}
