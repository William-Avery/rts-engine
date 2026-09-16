use game_types::{GameError, GameResult, SessionId};
use sim_core::command::Command;
use std::collections::BTreeMap;
use std::fmt;

/// A discrete privileged capability.
///
/// Debug and admin "cheats" are server-permissioned, never client-asserted: a
/// client can send an admin command envelope, but the server decides whether
/// the session holds the permission before the command ever reaches the
/// simulation.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum AdminPermission {
    /// Remove a session from the match.
    KickSession,
    /// Remove a session and mark it banned.
    BanSession,
    /// Override a session's anti-cheat trust level.
    SetTrustLevel,
    /// Grant resources directly into an inventory (debug cheat).
    GrantResources,
    /// Change another session's admin role.
    SetSessionRole,
    /// Read the server security log.
    InspectSecurityLog,
    /// Reload / re-pin the server content manifest.
    ReloadManifest,
    /// Switch the active anti-cheat provider.
    ToggleAntiCheat,
}

impl AdminPermission {
    pub const fn as_str(&self) -> &'static str {
        match self {
            AdminPermission::KickSession => "kick_session",
            AdminPermission::BanSession => "ban_session",
            AdminPermission::SetTrustLevel => "set_trust_level",
            AdminPermission::GrantResources => "grant_resources",
            AdminPermission::SetSessionRole => "set_session_role",
            AdminPermission::InspectSecurityLog => "inspect_security_log",
            AdminPermission::ReloadManifest => "reload_manifest",
            AdminPermission::ToggleAntiCheat => "toggle_anti_cheat",
        }
    }

    /// Stable wire code for telemetry and protocol use.
    pub const fn code(&self) -> u8 {
        match self {
            AdminPermission::KickSession => 1,
            AdminPermission::BanSession => 2,
            AdminPermission::SetTrustLevel => 3,
            AdminPermission::GrantResources => 4,
            AdminPermission::SetSessionRole => 5,
            AdminPermission::InspectSecurityLog => 6,
            AdminPermission::ReloadManifest => 7,
            AdminPermission::ToggleAntiCheat => 8,
        }
    }

    pub const fn from_code(code: u8) -> Option<AdminPermission> {
        match code {
            1 => Some(AdminPermission::KickSession),
            2 => Some(AdminPermission::BanSession),
            3 => Some(AdminPermission::SetTrustLevel),
            4 => Some(AdminPermission::GrantResources),
            5 => Some(AdminPermission::SetSessionRole),
            6 => Some(AdminPermission::InspectSecurityLog),
            7 => Some(AdminPermission::ReloadManifest),
            8 => Some(AdminPermission::ToggleAntiCheat),
            _ => None,
        }
    }
}

impl fmt::Display for AdminPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Role granted to a session on this server.
///
/// Roles are a strict containment ladder: every role holds every permission of
/// the role below it. [`AdminRegistry::authorize`] relies on that, and
/// `test_roles_form_a_containment_ladder` pins it.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub enum AdminRole {
    /// An ordinary player. Holds no privileged capability at all.
    #[default]
    Player,
    /// Can remove disruptive sessions and read security telemetry.
    Moderator,
    /// The match host. Full match control, including debug grants.
    Host,
    /// The server operator. Everything, including changing the anti-cheat provider.
    ServerOwner,
}

const MODERATOR_PERMISSIONS: &[AdminPermission] = &[
    AdminPermission::KickSession,
    AdminPermission::InspectSecurityLog,
];

const HOST_PERMISSIONS: &[AdminPermission] = &[
    AdminPermission::KickSession,
    AdminPermission::InspectSecurityLog,
    AdminPermission::BanSession,
    AdminPermission::SetTrustLevel,
    AdminPermission::GrantResources,
    AdminPermission::SetSessionRole,
    AdminPermission::ReloadManifest,
];

const OWNER_PERMISSIONS: &[AdminPermission] = &[
    AdminPermission::KickSession,
    AdminPermission::InspectSecurityLog,
    AdminPermission::BanSession,
    AdminPermission::SetTrustLevel,
    AdminPermission::GrantResources,
    AdminPermission::SetSessionRole,
    AdminPermission::ReloadManifest,
    AdminPermission::ToggleAntiCheat,
];

impl AdminRole {
    pub const fn permissions(&self) -> &'static [AdminPermission] {
        match self {
            AdminRole::Player => &[],
            AdminRole::Moderator => MODERATOR_PERMISSIONS,
            AdminRole::Host => HOST_PERMISSIONS,
            AdminRole::ServerOwner => OWNER_PERMISSIONS,
        }
    }

    pub fn allows(&self, permission: AdminPermission) -> bool {
        self.permissions().contains(&permission)
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            AdminRole::Player => "player",
            AdminRole::Moderator => "moderator",
            AdminRole::Host => "host",
            AdminRole::ServerOwner => "server-owner",
        }
    }

    pub const fn code(&self) -> u8 {
        match self {
            AdminRole::Player => 0,
            AdminRole::Moderator => 1,
            AdminRole::Host => 2,
            AdminRole::ServerOwner => 3,
        }
    }

    pub const fn from_code(code: u8) -> Option<AdminRole> {
        match code {
            0 => Some(AdminRole::Player),
            1 => Some(AdminRole::Moderator),
            2 => Some(AdminRole::Host),
            3 => Some(AdminRole::ServerOwner),
            _ => None,
        }
    }
}

impl From<AdminRole> for sim_core::dispatch::AdminRoleCode {
    /// Hand the role the server already authorized to the dispatcher.
    ///
    /// `sim-core` must not depend on `anti-cheat`, so the ladder stays owned
    /// here and only its stable wire code crosses the boundary. This is the one
    /// conversion point, so there is still exactly one permission model.
    fn from(role: AdminRole) -> Self {
        sim_core::dispatch::AdminRoleCode::new(role.code())
    }
}

impl fmt::Display for AdminRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Server-side authoritative role assignment and authorization.
///
/// Sessions are `Player` unless explicitly granted otherwise, so a brand new
/// connection can never execute a privileged command.
#[derive(Clone, Default, Debug)]
pub struct AdminRegistry {
    roles: BTreeMap<SessionId, AdminRole>,
    granted: u64,
    denied: u64,
}

impl AdminRegistry {
    pub fn new() -> Self {
        AdminRegistry::default()
    }

    /// Grant a role to a session.
    pub fn set_role(&mut self, session: SessionId, role: AdminRole) {
        if role == AdminRole::Player {
            self.roles.remove(&session);
        } else {
            self.roles.insert(session, role);
        }
    }

    /// Role held by a session. Unknown sessions are ordinary players.
    pub fn role(&self, session: SessionId) -> AdminRole {
        self.roles.get(&session).copied().unwrap_or_default()
    }

    /// Drop all state for a session (called on disconnect).
    pub fn remove_session(&mut self, session: SessionId) {
        self.roles.remove(&session);
    }

    /// Authorize a privileged action. This is the single authorization choke point.
    pub fn authorize(&mut self, session: SessionId, permission: AdminPermission) -> GameResult<()> {
        if self.role(session).allows(permission) {
            self.granted += 1;
            Ok(())
        } else {
            self.denied += 1;
            Err(GameError::PermissionDenied)
        }
    }

    /// Non-mutating authorization check, for UI and dry runs.
    pub fn can(&self, session: SessionId, permission: AdminPermission) -> bool {
        self.role(session).allows(permission)
    }

    pub fn granted_count(&self) -> u64 {
        self.granted
    }

    pub fn denied_count(&self) -> u64 {
        self.denied
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SessionId, &AdminRole)> {
        self.roles.iter()
    }
}

/// Permission a command requires, or `None` if it is an ordinary gameplay command.
///
/// This is the mapping the server consults before a command is buffered. Keeping
/// it here (rather than inline in the server) means the privileged command set
/// is enumerated in exactly one place.
pub const fn required_admin_permission(command: &Command) -> Option<AdminPermission> {
    match command {
        Command::AdminKickSession { .. } => Some(AdminPermission::KickSession),
        Command::AdminSetTrustLevel { .. } => Some(AdminPermission::SetTrustLevel),
        Command::AdminGrantResource { .. } => Some(AdminPermission::GrantResources),
        Command::AdminSetSessionRole { .. } => Some(AdminPermission::SetSessionRole),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{EntityId, ResourceId};

    #[test]
    fn test_default_session_is_an_unprivileged_player() {
        let reg = AdminRegistry::new();
        assert_eq!(reg.role(SessionId::new(1)), AdminRole::Player);
        for permission in OWNER_PERMISSIONS {
            assert!(!reg.can(SessionId::new(1), *permission), "{permission}");
        }
    }

    #[test]
    fn test_roles_form_a_containment_ladder() {
        for permission in AdminRole::Moderator.permissions() {
            assert!(
                AdminRole::Host.allows(*permission),
                "host < mod: {permission}"
            );
        }
        for permission in AdminRole::Host.permissions() {
            assert!(
                AdminRole::ServerOwner.allows(*permission),
                "owner < host: {permission}"
            );
        }
    }

    #[test]
    fn test_moderator_cannot_grant_resources_or_change_roles() {
        let mut reg = AdminRegistry::new();
        reg.set_role(SessionId::new(2), AdminRole::Moderator);
        assert!(reg.can(SessionId::new(2), AdminPermission::KickSession));
        assert!(!reg.can(SessionId::new(2), AdminPermission::GrantResources));
        assert!(!reg.can(SessionId::new(2), AdminPermission::SetSessionRole));
        assert!(!reg.can(SessionId::new(2), AdminPermission::ToggleAntiCheat));
    }

    #[test]
    fn test_only_server_owner_may_toggle_anti_cheat() {
        let mut reg = AdminRegistry::new();
        reg.set_role(SessionId::new(3), AdminRole::Host);
        assert!(!reg.can(SessionId::new(3), AdminPermission::ToggleAntiCheat));
        reg.set_role(SessionId::new(3), AdminRole::ServerOwner);
        assert!(reg.can(SessionId::new(3), AdminPermission::ToggleAntiCheat));
    }

    #[test]
    fn test_authorize_denies_unprivileged_and_counts_outcomes() {
        let mut reg = AdminRegistry::new();
        reg.set_role(SessionId::new(1), AdminRole::Host);
        assert!(
            reg.authorize(SessionId::new(1), AdminPermission::GrantResources)
                .is_ok()
        );
        assert_eq!(
            reg.authorize(SessionId::new(2), AdminPermission::GrantResources),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(reg.granted_count(), 1);
        assert_eq!(reg.denied_count(), 1);
    }

    #[test]
    fn test_removing_a_session_revokes_its_role() {
        let mut reg = AdminRegistry::new();
        reg.set_role(SessionId::new(1), AdminRole::ServerOwner);
        reg.remove_session(SessionId::new(1));
        assert_eq!(reg.role(SessionId::new(1)), AdminRole::Player);
    }

    #[test]
    fn test_ordinary_gameplay_commands_require_no_permission() {
        let cmd = Command::TransferResource {
            from_entity: EntityId::new(1),
            to_entity: EntityId::new(2),
            resource_id: ResourceId::new(1),
            amount: 5,
        };
        assert_eq!(required_admin_permission(&cmd), None);
    }

    #[test]
    fn test_admin_commands_map_to_their_permission() {
        assert_eq!(
            required_admin_permission(&Command::AdminKickSession {
                target_session: SessionId::new(2),
                reason_code: 1
            }),
            Some(AdminPermission::KickSession)
        );
        assert_eq!(
            required_admin_permission(&Command::AdminSetTrustLevel {
                target_session: SessionId::new(2),
                trust_code: 2
            }),
            Some(AdminPermission::SetTrustLevel)
        );
        assert_eq!(
            required_admin_permission(&Command::AdminGrantResource {
                target_entity: EntityId::new(1),
                resource_id: ResourceId::new(1),
                amount: 10
            }),
            Some(AdminPermission::GrantResources)
        );
        assert_eq!(
            required_admin_permission(&Command::AdminSetSessionRole {
                target_session: SessionId::new(2),
                role_code: 1
            }),
            Some(AdminPermission::SetSessionRole)
        );
    }

    #[test]
    fn test_permission_and_role_wire_codes_roundtrip() {
        for permission in OWNER_PERMISSIONS {
            assert_eq!(
                AdminPermission::from_code(permission.code()),
                Some(*permission)
            );
        }
        for role in [
            AdminRole::Player,
            AdminRole::Moderator,
            AdminRole::Host,
            AdminRole::ServerOwner,
        ] {
            assert_eq!(AdminRole::from_code(role.code()), Some(role));
        }
        assert_eq!(AdminPermission::from_code(200), None);
        assert_eq!(AdminRole::from_code(200), None);
    }
}
