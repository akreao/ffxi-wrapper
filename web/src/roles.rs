//! Website roles: stored per account in the site's own database (`site_roles`) and set only
//! by an owner. In-game GM levels (`chars.gmlevel`, one per character) give no website role.

use crate::auth::{self, Role};
use crate::game::DbResult;
use crate::state::AppState;
use std::sync::atomic::Ordering;

/// Copies the roles GM levels used to give (5 Owner, 3 and 4 Moderator, from the highest
/// level among an account's characters) into `site_roles`, once ever, so nobody loses
/// access on upgrade. Cheap after the first success.
pub async fn ensure_seeded(state: &AppState) -> DbResult<()> {
    if state.roles_seeded.load(Ordering::Acquire) {
        return Ok(());
    }
    if !state.store.site_roles_seeded().await? {
        let rows: Vec<(i64, String, &str)> = state
            .game
            .staff_by_gm_level()
            .await?
            .into_iter()
            .filter_map(|(accid, login, gm)| match auth::legacy_role_for_gm(gm) {
                Role::Player | Role::Visitor => None,
                r => Some((accid, login, r.as_str())),
            })
            .collect();
        if state.store.seed_site_roles(&rows, state.now()).await? {
            tracing::info!("copied {} website roles from GM levels (one time)", rows.len());
        }
    }
    state.roles_seeded.store(true, Ordering::Release);
    Ok(())
}

/// The website role of the account `accid` whose login is now `login`.
pub async fn role_of(state: &AppState, accid: i64, login: &str) -> DbResult<Role> {
    ensure_seeded(state).await?;
    let row = state.store.site_role(accid).await?;
    Ok(auth::stored_role(row.as_ref().map(|r| (r.role.as_str(), r.login.as_str())), login))
}
