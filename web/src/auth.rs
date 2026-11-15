//! Roles, the rules for logins and passwords, password hashes, two-step
//! codes and session lifetimes. Pure logic: no database, no HTTP.

use crate::store::{Session, SessionKind};
use totp_rs::{Algorithm, Builder, Secret};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Visitor,
    Player,
    Moderator,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Visitor => "visitor",
            Role::Player => "player",
            Role::Moderator => "moderator",
            Role::Owner => "owner",
        }
    }

    pub fn is_admin(self) -> bool {
        self >= Role::Moderator
    }
}

impl Role {
    /// A stored role name; anything else is a Player.
    pub fn from_stored(s: &str) -> Role {
        match s {
            "owner" => Role::Owner,
            "moderator" => Role::Moderator,
            _ => Role::Player,
        }
    }

    /// A role an owner may give: player, moderator or owner.
    pub fn assignable(s: &str) -> Option<Role> {
        match s {
            "player" => Some(Role::Player),
            "moderator" => Some(Role::Moderator),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }
}

/// The role an account gets from its stored row, if any. The row must name the account's
/// current login: a role never passes to a different account that later gets the same id.
pub fn stored_role(row: Option<(&str, &str)>, login: &str) -> Role {
    match row {
        Some((role, stored_login)) if stored_login == login => Role::from_stored(role),
        _ => Role::Player,
    }
}

/// The role the site used to work out from the highest GM level among an account's
/// characters (5 Owner, 3 and 4 Moderator). Now only used once, to copy those roles into
/// the site's own table on upgrade; GM levels no longer give website roles.
pub fn legacy_role_for_gm(gm: i64) -> Role {
    match gm {
        5.. => Role::Owner,
        3 | 4 => Role::Moderator,
        _ => Role::Player,
    }
}

/// Who is acting, for the "never act on someone at your own role or above" rule.
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    pub role: Role,
    pub owner_link: bool,
}

/// May `actor` change an account whose website role is `target`? Only when that role is
/// below their own; the owner link (which has no account of its own) may act on anyone.
pub fn can_act_on(actor: Actor, target: Role) -> bool {
    if actor.owner_link {
        return true;
    }
    actor.role.is_admin() && target < actor.role
}

// ---- logins and passwords, as LSB checks them ----

pub const LOGIN_MAX: usize = 16;
pub const PASSWORD_MAX: usize = 32;
pub const PASSWORD_MIN_NEW: usize = 8;
/// Cost LSB writes (`$2a$12$`).
pub const BCRYPT_COST: u32 = 12;

fn has_control(s: &str) -> bool {
    s.bytes().any(|b| b < 0x20)
}

/// `isStringMalformed(login, 16)`: 1 to 16 bytes, none below 0x20.
pub fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= LOGIN_MAX && !has_control(login)
}

/// A password to check: 1 to 32 bytes.
pub fn valid_password(pw: &str) -> bool {
    !pw.is_empty() && pw.len() <= PASSWORD_MAX
}

/// A password to set: also no byte below 0x20, and at least 8 characters.
pub fn valid_new_password(pw: &str) -> Result<(), &'static str> {
    if pw.len() > PASSWORD_MAX {
        return Err("The password can be at most 32 characters.");
    }
    if has_control(pw) {
        return Err("The password can't contain control characters.");
    }
    if pw.chars().count() < PASSWORD_MIN_NEW {
        return Err("Use at least 8 characters.");
    }
    Ok(())
}

// ---- password hashes ----

#[derive(Debug, PartialEq, Eq)]
pub enum HashCheck {
    /// Not a bcrypt hash this site can verify (for instance an old `PASSWORD()` hash
    /// that LSB upgrades at the next game login).
    Unsupported,
    Match,
    Mismatch,
}

/// 60 bytes starting `$2a$`, `$2b$` or `$2y$`. `$2x$` is refused: the bcrypt crate doesn't verify it.
pub fn is_supported_hash(hash: &str) -> bool {
    hash.len() == 60 && ["$2a$", "$2b$", "$2y$"].iter().any(|p| hash.starts_with(p))
}

pub fn check_password(password: &str, hash: &str) -> HashCheck {
    if !is_supported_hash(hash) {
        return HashCheck::Unsupported;
    }
    match bcrypt::verify(password, hash) {
        Ok(true) => HashCheck::Match,
        _ => HashCheck::Mismatch,
    }
}

/// A new hash in the form LSB writes: `$2a$` and cost 12 (a test may ask for less).
pub fn hash_password(password: &str, cost: u32) -> Result<String, bcrypt::BcryptError> {
    Ok(bcrypt::hash_with_result(password, cost)?.format_for_version(bcrypt::Version::TwoA))
}

// ---- two-step codes ----

/// Checks a 6-digit code against LSB's base32 secret: SHA-1, 30 s steps, one step either
/// side. Returns the step that matched; the caller refuses a step it has seen.
pub fn check_totp(secret_base32: &str, code: &str, now: u64) -> Option<u64> {
    let secret = Secret::try_from_base32(secret_base32.trim()).ok()?;
    let totp = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(secret)
        .build_noncompliant();
    totp.check(code.trim(), now)
}

// ---- session lifetimes ----

const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

/// How long a session may sit unused: 30 minutes for staff, 7 days for players.
pub fn idle_limit(role: Role) -> i64 {
    if role.is_admin() { 30 * MIN } else { 7 * DAY }
}

/// How long a session lasts in all: 12 hours for staff, 30 days for players.
pub fn absolute_limit(role: Role) -> i64 {
    if role.is_admin() { 12 * HOUR } else { 30 * DAY }
}

/// Is the session still good for someone with this role? The role is the current one, so
/// a promotion shortens the session at once.
pub fn session_alive(role: Role, s: &Session, now: i64) -> bool {
    let idle_ok = now - s.last_seen <= idle_limit(role);
    let abs_end = s.expires.min(s.created + absolute_limit(role));
    idle_ok && now < abs_end
}

/// The lifetime to give a new session, from the role at sign-in.
pub fn new_session_lifetime(role: Role, kind: &SessionKind) -> i64 {
    match kind {
        SessionKind::Owner => absolute_limit(Role::Owner),
        SessionKind::Account => absolute_limit(role),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_come_from_the_stored_row_for_the_same_login() {
        assert_eq!(stored_role(Some(("owner", "steven")), "steven"), Role::Owner);
        assert_eq!(stored_role(Some(("moderator", "halvard")), "halvard"), Role::Moderator);
        assert_eq!(stored_role(Some(("owner", "steven")), "someoneelse"), Role::Player);
        assert_eq!(stored_role(Some(("admin", "x")), "x"), Role::Player);
        assert_eq!(stored_role(None, "x"), Role::Player);
        assert!(Role::Owner > Role::Moderator && Role::Moderator > Role::Player && Role::Player > Role::Visitor);
        assert!(!Role::Player.is_admin() && Role::Moderator.is_admin());
        assert_eq!(Role::assignable("visitor"), None);
    }

    #[test]
    fn the_one_time_copy_maps_gm_levels_as_before() {
        let roles: Vec<Role> = (0..=5).map(legacy_role_for_gm).collect();
        assert_eq!(roles, [Role::Player, Role::Player, Role::Player, Role::Moderator, Role::Moderator, Role::Owner]);
    }

    #[test]
    fn nobody_acts_on_an_equal_or_higher_role_except_the_owner_link() {
        let moderator = Actor { role: Role::Moderator, owner_link: false };
        assert!(can_act_on(moderator, Role::Player));
        assert!(!can_act_on(moderator, Role::Moderator));
        assert!(!can_act_on(moderator, Role::Owner));
        let owner = Actor { role: Role::Owner, owner_link: false };
        assert!(can_act_on(owner, Role::Moderator));
        assert!(!can_act_on(owner, Role::Owner));
        let link = Actor { role: Role::Owner, owner_link: true };
        assert!(can_act_on(link, Role::Owner));
        let player = Actor { role: Role::Player, owner_link: false };
        assert!(!can_act_on(player, Role::Player));
    }

    #[test]
    fn login_and_password_lengths_are_lsbs() {
        assert!(valid_login("a") && valid_login(&"a".repeat(16)));
        assert!(!valid_login("") && !valid_login(&"a".repeat(17)));
        assert!(!valid_login("bad\u{1f}name") && valid_login("with space"));
        assert!(valid_password("x") && valid_password(&"p".repeat(32)));
        assert!(!valid_password("") && !valid_password(&"p".repeat(33)));
        // Multi-byte characters count as bytes.
        assert!(!valid_login(&"é".repeat(9)));
    }

    #[test]
    fn new_passwords_have_their_own_rules() {
        assert!(valid_new_password("longenough").is_ok());
        assert!(valid_new_password("short").is_err());
        assert!(valid_new_password(&"p".repeat(33)).is_err());
        assert!(valid_new_password("tab\tinside1").is_err());
    }

    #[test]
    fn bcrypt_hashes_are_lsbs_form_and_odd_ones_are_refused() {
        let h = hash_password("hunter22", 4).unwrap();
        assert!(h.starts_with("$2a$04$") && h.len() == 60, "{h}");
        assert_eq!(check_password("hunter22", &h), HashCheck::Match);
        assert_eq!(check_password("hunter23", &h), HashCheck::Mismatch);
        // $2b$ and $2y$ spellings of the same hash verify; $2x$ and old PASSWORD() hashes don't.
        let b = h.replacen("$2a$", "$2b$", 1);
        assert_eq!(check_password("hunter22", &b), HashCheck::Match);
        let x = h.replacen("$2a$", "$2x$", 1);
        assert_eq!(check_password("hunter22", &x), HashCheck::Unsupported);
        assert_eq!(check_password("hunter22", "*6BB4837EB74329105EE4568DDA7DC67ED2CA2AD9"), HashCheck::Unsupported);
        assert_eq!(check_password("hunter22", ""), HashCheck::Unsupported);
    }

    #[test]
    fn a_hash_made_by_lsb_verifies() {
        // Made with the same library at cost 12 by the server's `bcrypt` call for the
        // password "lsb-test-password" (a scratch value, not a real account).
        let h = hash_password("lsb-test-password", 12).unwrap();
        assert!(h.starts_with("$2a$12$"));
        assert_eq!(check_password("lsb-test-password", &h), HashCheck::Match);
    }

    // RFC 6238, appendix B, SHA-1: secret "12345678901234567890", 8 digits there; the
    // 6-digit codes are the last six digits of those.
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn totp_matches_the_rfc_6238_vectors() {
        assert_eq!(check_totp(RFC_SECRET, "287082", 59), Some(1));
        assert_eq!(check_totp(RFC_SECRET, "081804", 1111111109), Some(37037036));
        assert_eq!(check_totp(RFC_SECRET, "050471", 1111111111), Some(37037037));
        assert_eq!(check_totp(RFC_SECRET, "005924", 1234567890), Some(41152263));
    }

    #[test]
    fn totp_allows_one_step_either_side_but_not_two() {
        // The code for step 1 (t = 30..59) is 287082.
        assert_eq!(check_totp(RFC_SECRET, "287082", 59 + 30), Some(1), "one step late");
        assert_eq!(check_totp(RFC_SECRET, "287082", 30 - 30), Some(1), "one step early");
        assert_eq!(check_totp(RFC_SECRET, "287082", 59 + 60), None, "two steps late");
        assert_eq!(check_totp(RFC_SECRET, "287082", 59 + 120), None);
        assert_eq!(check_totp(RFC_SECRET, "12345", 59), None, "five digits");
        assert_eq!(check_totp(RFC_SECRET, "abcdef", 59), None);
        assert_eq!(check_totp("not base32!", "287082", 59), None);
    }

    fn session(created: i64, last_seen: i64, expires: i64) -> Session {
        Session { id_hash: vec![], kind: SessionKind::Account, accid: Some(1), login: None, csrf: String::new(), created, last_seen, expires }
    }

    #[test]
    fn sessions_end_by_idle_time_and_by_age() {
        let s = session(0, 0, 30 * DAY);
        assert!(session_alive(Role::Player, &s, 6 * DAY));
        assert!(!session_alive(Role::Player, &s, 8 * DAY));
        // Staff: 30 minutes idle, 12 hours in all.
        assert!(session_alive(Role::Moderator, &s, 29 * MIN));
        assert!(!session_alive(Role::Moderator, &s, 31 * MIN));
        let busy = session(0, 11 * HOUR + 50 * MIN, 30 * DAY);
        assert!(session_alive(Role::Owner, &busy, 11 * HOUR + 55 * MIN));
        assert!(!session_alive(Role::Owner, &busy, 12 * HOUR + MIN), "past the 12-hour cap");
        // A player session past its own end.
        assert!(!session_alive(Role::Player, &session(0, 29 * DAY, 29 * DAY), 29 * DAY + 1));
    }
}
