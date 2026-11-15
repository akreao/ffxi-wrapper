//! Website roles: stored per account in the site's own database (`site_roles`) and set only
//! by an owner. In-game GM levels (`chars.gmlevel`, one per character) give no website role.

use crate::auth::{self, Actor, Role};
use crate::game::DbResult;
use crate::http::{ApiError, ApiResult, Identity};
use crate::model::Standing;
use crate::state::AppState;
use std::sync::atomic::Ordering;
use tokio::sync::MutexGuard;

/// On a store upgraded from before website roles, copies the roles GM levels used to give
/// (5 Owner, 3 and 4 Moderator, from the highest level among an account's characters) into
/// `site_roles`, once ever, so nobody loses access. A new store is marked done when it is
/// made, so it copies nothing. Cheap after the first success.
pub async fn ensure_seeded(state: &AppState) -> DbResult<()> {
    if state.roles_seeded.load(Ordering::Acquire) {
        return Ok(());
    }
    if !state.store.site_roles_seeded().await? {
        let rows: Vec<(i64, String, Option<i64>, &str)> = state
            .game
            .staff_by_gm_level()
            .await?
            .into_iter()
            .filter_map(|(accid, login, created, gm)| match auth::legacy_role_for_gm(gm) {
                Role::Player | Role::Visitor => None,
                r => Some((accid, login, created, r.as_str())),
            })
            .collect();
        if state.store.seed_site_roles(&rows, state.now()).await? {
            tracing::info!("copied {} website roles from GM levels (one time)", rows.len());
        }
    }
    state.roles_seeded.store(true, Ordering::Release);
    Ok(())
}

/// The website role of the account `accid`, as it stands now.
pub async fn role_of(state: &AppState, accid: i64, now: &Standing) -> DbResult<Role> {
    ensure_seeded(state).await?;
    let row = state.store.site_role(accid).await?;
    Ok(auth::stored_role(row.as_ref().map(|r| (r.role.as_str(), r.login.as_str(), r.created)), &now.login, now.created))
}

/// The website role of the account `accid`, reading it afresh; Player when it is gone.
pub async fn account_role(state: &AppState, accid: i64) -> DbResult<Role> {
    match state.game.standing(accid).await? {
        Some(s) => role_of(state, accid, &s).await,
        None => Ok(Role::Player),
    }
}

/// Takes `AppState::role_gate` for a change, then reads the acting account's own role again
/// under it (a demotion or ban that landed since the request began counts), and refuses
/// unless it is at least `min`. Hold the guard until the change is written.
pub async fn lock<'a>(state: &'a AppState, id: &Identity, min: Role) -> ApiResult<(MutexGuard<'a, ()>, Actor)> {
    let gate = state.role_gate.lock().await;
    let role = if id.owner_link() {
        Role::Owner
    } else {
        let accid = id.accid().ok_or_else(ApiError::unauthorized)?;
        match state.game.standing(accid).await? {
            Some(s) if s.status & 1 == 1 => role_of(state, accid, &s).await?,
            _ => return Err(ApiError::unauthorized()),
        }
    };
    if role < min {
        return Err(ApiError::forbidden());
    }
    Ok((gate, Actor { role, owner_link: id.owner_link() }))
}
