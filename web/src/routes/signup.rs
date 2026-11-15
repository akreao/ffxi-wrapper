//! Creating a game account on the site, and the owner's invite codes.
//!
//! The account is made exactly as LSB's login server makes one (`LOGIN_CREATE`), and only while
//! LSB's own `login.ACCOUNT_CREATION` is on. Order of a sign-up, so nothing leaks to someone
//! who hasn't earned it: closed? -> plain HTTP from the internet? -> input shape -> limits ->
//! bcrypt (always, so every answer takes as long) -> invite code -> login free? -> insert.
//! "That login is taken" is only ever said to someone holding a working invite code, or on a
//! site that asks for none.

use crate::auth::{self, Role};
use crate::http::*;
use crate::limits::SignupLimit;
use crate::routes::session::{bcrypt_slot, record};
use crate::state::Shared;
use crate::store::{self, NewAction, NewInvite, SessionKind};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/public/signup", get(info).post(create))
        .route("/admin/invites", get(invites).post(invite_new))
        .route("/admin/invites/{id}/revoke", post(invite_revoke))
        .route("/admin/signup", get(settings).put(set_settings))
}

/// The keys of `GET /api/public/signup`. A test pins this list.
pub const SIGNUP_KEYS: [&str; 6] = ["invite_required", "login_max", "login_min", "open", "password_max", "password_min"];

pub const LOGIN_MIN: usize = 3;
pub const LOGIN_MAX: usize = 16;

/// Letters and digits only, 3 to 16: stricter than LSB (1 to 16 bytes, no control bytes),
/// which only turns away names LSB would have taken.
pub fn valid_signup_login(login: &str) -> bool {
    (LOGIN_MIN..=LOGIN_MAX).contains(&login.len()) && login.bytes().all(|b| b.is_ascii_alphanumeric())
}

// ---- the public side ----

fn limited(state: &Shared, ip: &str) -> ApiResult<()> {
    if state.limiter.public_get_allowed(ip, state.now()) {
        Ok(())
    } else {
        Err(ApiError::too_many("Too many requests. Try again in a minute."))
    }
}

async fn info(State(state): State<Shared>, ClientIp(ip): ClientIp) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    Ok(Json(json!({
        "open": state.signup_open().await,
        "invite_required": state.store.invite_required().await?,
        "login_min": LOGIN_MIN, "login_max": LOGIN_MAX,
        "password_min": 8, "password_max": 32,
    })))
}

#[derive(Deserialize)]
struct SignupBody {
    #[serde(default)]
    login: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    invite: String,
}

const TAKEN: &str = "That login is already used. Pick another.";
const BAD_INVITE: &str = "That invite code can't be used.";

async fn create(State(state): State<Shared>, ClientIp(ip): ClientIp, headers: HeaderMap, Json(body): Json<SignupBody>) -> ApiResult<Response> {
    if !state.signup_open().await {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "Sign-up is closed on this server.").with(json!({ "closed": true })));
    }
    // What can be said about the input alone, before anything is counted or looked up.
    if !valid_signup_login(&body.login) {
        return Err(ApiError::bad_request("The login must be 3 to 16 letters or numbers."));
    }
    auth::valid_new_password(&body.password).map_err(ApiError::bad_request)?;
    let needs_invite = state.store.invite_required().await?;
    let code = store::normalize_code(&body.invite);
    if needs_invite && code.is_none() {
        return Err(ApiError::bad_request("Enter an invite code."));
    }

    // Everything from here runs to its end even when the visitor closes the page: a dropped
    // request would otherwise leave a counted slot, a spent invite or a held lock behind.
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(String::from);
    let invite = needs_invite.then(|| store::code_hash(code.as_deref().unwrap_or("")));
    tokio::spawn(finish(state, ip, body.login, body.password, invite, ua)).await.unwrap_or_else(|_| Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server.")))
}

async fn hash_for_signup(state: &Shared, password: &str) -> ApiResult<String> {
    let _slot = bcrypt_slot(state).await?;
    let (pw, cost) = (password.to_string(), state.bcrypt_cost);
    tokio::task::spawn_blocking(move || auth::hash_password(&pw, cost))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server."))
}

/// No account came of this sign-up: the day's count and any invite use are given back.
async fn give_back(state: &Shared, invite_id: Option<i64>) {
    state.limiter.signup_not_created();
    if let Some(id) = invite_id {
        if let Err(e) = state.store.invite_return(id).await {
            tracing::warn!("couldn't give an invite use back: {e}");
        }
    }
}

/// Counts the attempt, takes the invite use, hashes, makes the account, signs it in. A use or
/// day slot that bought no account goes back. The invite is checked before any bcrypt work, so
/// guessing codes costs the site nothing; a refused code waits as long as a hash would take
/// (without using the CPU), so the answer doesn't tell a good code from a bad one by speed.
async fn finish(state: Shared, ip: String, login: String, password: String, invite: Option<Vec<u8>>, ua: Option<String>) -> ApiResult<Response> {
    let now = state.now();
    if let Err(which) = state.limiter.reserve_signup(&ip, now) {
        return Err(ApiError::too_many(match which {
            SignupLimit::Address => "Too many sign-ups from your address. Try again in an hour.",
            SignupLimit::Day => "Sign-ups are full for today. Try again tomorrow.",
            SignupLimit::Attempts => "Sign-up is busy. Try again in a while.",
        }));
    }
    let mut invite_id = None;
    if let Some(code_hash) = &invite {
        match state.store.invite_take(code_hash, now).await {
            Ok(Some(id)) => invite_id = Some(id),
            other => {
                state.limiter.signup_not_created();
                if let Err(e) = other {
                    return Err(e.into());
                }
                tokio::time::sleep(std::time::Duration::from_micros(61u64 << state.bcrypt_cost.min(16))).await;
                record(&state, NewAction { actor_kind: "visitor", actor_role: "visitor", ip: Some(ip), action: "signup_refused", reason: Some("invite code refused".into()), result: "refused", ..Default::default() }).await;
                return Err(ApiError::bad_request(BAD_INVITE));
            }
        }
    }
    let hash = match hash_for_signup(&state, &password).await {
        Ok(h) => h,
        Err(e) => {
            give_back(&state, invite_id).await;
            state.limiter.signup_undo(&ip);
            return Err(e);
        }
    };
    let accid = match state.game.create_account(&login, &hash).await {
        Ok(crate::model::Created::Account(id)) => id,
        Ok(crate::model::Created::Taken) => {
            give_back(&state, invite_id).await;
            return Err(ApiError::new(StatusCode::CONFLICT, TAKEN));
        }
        Err(e) => {
            give_back(&state, invite_id).await;
            return Err(e.into());
        }
    };

    let detail = json!({ "invite": invite_id });
    record(
        &state,
        NewAction {
            actor_kind: "visitor",
            actor_role: "visitor",
            ip: Some(ip.clone()),
            action: "signup",
            target_kind: Some("account"),
            target_id: Some(accid),
            target_name: Some(login.clone()),
            detail: Some(detail.to_string()),
            result: "ok",
            ..Default::default()
        },
    )
    .await;

    // Signed in at once, as a Player (a new account has no website role).
    let lifetime = auth::new_session_lifetime(Role::Player, &SessionKind::Account);
    let (cookie, csrf) = state
        .store
        .create_session(crate::store::NewSession { kind: SessionKind::Account, accid: Some(accid), login: Some(&login), ip: &ip, user_agent: ua.as_deref(), now, lifetime })
        .await?;
    let mut res = (StatusCode::CREATED, Json(json!({ "ok": true, "role": "player", "csrf": csrf, "next": "/me?welcome=1" }))).into_response();
    res.headers_mut().insert(header::SET_COOKIE, set_cookie(&state, &cookie, lifetime));
    Ok(res)
}

// ---- the owner's side ----

/// The base of an action-log row for something the owner did about sign-up.
fn act(id: &Identity, ip: &str, action: &'static str, target: Option<i64>, name: &str, detail: Value) -> NewAction {
    NewAction {
        actor_kind: if id.owner_link() { "owner" } else { "account" },
        actor_accid: id.accid(),
        actor_login: id.login().map(String::from),
        actor_role: id.role.as_str(),
        ip: Some(ip.to_string()),
        action,
        target_kind: Some("invite"),
        target_id: target,
        target_name: Some(name.to_string()),
        detail: Some(detail.to_string()),
        result: "ok",
        ..Default::default()
    }
}

fn who(id: &Identity) -> String {
    id.login().unwrap_or("Owner link").to_string()
}

fn invite_state(i: &store::Invite, now: i64) -> &'static str {
    if i.revoked {
        "revoked"
    } else if i.expires.is_some_and(|e| e <= now) {
        "expired"
    } else if i.uses >= i.max_uses {
        "used up"
    } else {
        "active"
    }
}

fn invite_json(i: &store::Invite, now: i64) -> Value {
    json!({
        "id": i.id, "hint": i.hint, "note": i.note, "max_uses": i.max_uses, "uses": i.uses,
        "expires": i.expires, "created": i.created, "created_by": i.created_by,
        "state": invite_state(i, now),
    })
}

async fn overview(state: &Shared) -> ApiResult<Value> {
    let now = state.now();
    Ok(json!({
        "items": state.store.invites().await?.iter().map(|i| invite_json(i, now)).collect::<Vec<_>>(),
        "invite_required": state.store.invite_required().await?,
        "lsb_open": state.signup_open().await,
        "max_note": MAX_NOTE, "max_uses": MAX_USES,
    }))
}

async fn invites(State(state): State<Shared>, Owner(_): Owner) -> ApiResult<Json<Value>> {
    Ok(Json(overview(&state).await?))
}

const MAX_NOTE: usize = 60;
const MAX_USES: i64 = 1000;
const MAX_DAYS: i64 = 365;
/// No 0, O, 1 or I: a code read out loud or copied by hand has no look-alikes.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// 16 characters of 5 bits: 80 bits, shown as `XXXX-XXXX-XXXX-XXXX`.
fn new_code() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system random source works");
    let chars: Vec<char> = b.iter().map(|x| CODE_ALPHABET[(x & 31) as usize] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

#[derive(Deserialize)]
struct InviteBody {
    #[serde(default)]
    note: String,
    #[serde(default = "one")]
    max_uses: i64,
    /// Days the code works; absent or 0 means no end.
    #[serde(default)]
    days: i64,
}

fn one() -> i64 {
    1
}

async fn invite_new(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Json(b): Json<InviteBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    let note = b.note.trim();
    if note.chars().count() > MAX_NOTE || note.chars().any(|c| c.is_control()) {
        return Err(ApiError::bad_request("The note must be one line of at most 60 characters."));
    }
    if !(1..=MAX_USES).contains(&b.max_uses) {
        return Err(ApiError::bad_request("Uses must be from 1 to 1000."));
    }
    if !(0..=MAX_DAYS).contains(&b.days) {
        return Err(ApiError::bad_request("Days must be from 0 (no end) to 365."));
    }
    let now = state.now();
    let code = new_code();
    let normalized = store::normalize_code(&code).expect("a made-up code is letters and digits");
    let expires = (b.days > 0).then(|| now + b.days * 86_400);
    let new_id = state
        .store
        .invite_create(NewInvite { code_hash: store::code_hash(&normalized), hint: &normalized[normalized.len() - 4..], note, max_uses: b.max_uses, expires, by: &who(&id), now })
        .await?;
    // The code itself is never logged.
    let detail = json!({ "note": note, "max_uses": b.max_uses, "expires": expires });
    record(&state, act(&id, &ip, "invite_create", Some(new_id), note, detail)).await;
    Ok((StatusCode::CREATED, Json(json!({ "id": new_id, "code": code }))))
}

async fn invite_revoke(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Path(invite_id): Path<i64>) -> ApiResult<Json<Value>> {
    let before = state.store.invite(invite_id).await?.ok_or_else(ApiError::not_found)?;
    if before.revoked {
        return Ok(Json(json!({ "revoked": invite_id })));
    }
    if !state.store.invite_revoke(invite_id).await? {
        return Err(ApiError::not_found());
    }
    record(&state, act(&id, &ip, "invite_revoke", Some(invite_id), &before.note, json!({ "uses": before.uses, "max_uses": before.max_uses }))).await;
    Ok(Json(json!({ "revoked": invite_id })))
}

async fn settings(State(state): State<Shared>, Owner(_): Owner) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "invite_required": state.store.invite_required().await?, "lsb_open": state.signup_open().await })))
}

#[derive(Deserialize)]
struct SettingsBody {
    invite_required: bool,
}

async fn set_settings(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Json(b): Json<SettingsBody>) -> ApiResult<Json<Value>> {
    let before = state.store.invite_required().await?;
    state.store.set_setting("signup_needs_invite", if b.invite_required { "1" } else { "0" }, &who(&id), state.now()).await?;
    if before != b.invite_required {
        let mut a = act(&id, &ip, "signup_setting", None, "Sign-up needs an invite code", json!({ "before": before, "after": b.invite_required }));
        a.target_kind = Some("setting");
        record(&state, a).await;
    }
    Ok(Json(json!({ "invite_required": b.invite_required })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logins_are_three_to_sixteen_letters_and_digits() {
        for ok in ["abc", "Wren", "a1b2c3d4e5f6g7h8"] {
            assert!(valid_signup_login(ok), "{ok}");
        }
        for bad in ["", "ab", "a1b2c3d4e5f6g7h89", "wren!", "wr en", "wrén", "wren\n", "a-b"] {
            assert!(!valid_signup_login(bad), "{bad:?}");
        }
    }

    #[test]
    fn made_up_codes_have_no_look_alikes_and_survive_a_round_trip() {
        let c = new_code();
        assert_eq!(c.len(), 19);
        assert!(c.chars().all(|ch| ch == '-' || CODE_ALPHABET.contains(&(ch as u8))));
        let n = store::normalize_code(&c).unwrap();
        assert_eq!(n.len(), 16);
        assert_eq!(store::normalize_code(&c.to_lowercase().replace('-', " ")).unwrap(), n);
        assert_ne!(new_code(), new_code());
    }
}
