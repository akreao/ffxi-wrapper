//! The owner's editors for news posts and site content, and the Markdown preview.
//!
//! Owner only: every handler takes the `Owner` extractor, so a moderator or player is
//! refused before anything runs. Every write goes to the action log with the sizes or
//! titles it changed (never a whole body) and drops the public cache.

use crate::content;
use crate::http::*;
use crate::markdown;
use crate::routes::session::record;
use crate::state::Shared;
use crate::store::{NewAction, NewsPost, PostFields};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub const MAX_TITLE: usize = 120;
pub const MAX_AUTHOR: usize = 40;
pub const MAX_BODY: usize = 20_000;
const DEFAULT_AUTHOR: &str = "Staff";

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/admin/news", get(list).post(create))
        .route("/admin/news/{id}", get(read).put(update).delete(remove))
        .route("/admin/content", get(blocks))
        .route("/admin/content/{key}", put(save_block).delete(reset_block))
        .route("/admin/preview", post(preview))
        // A post is far longer than the few short fields the rest of the API takes.
        .layer(DefaultBodyLimit::max(96 * 1024))
}

/// The base of an action-log row for something the owner did to news or content.
fn act(id: &Identity, ip: &str, action: &'static str, kind: &'static str, target: Option<i64>, name: &str, detail: Value) -> NewAction {
    NewAction {
        actor_kind: if id.owner_link() { "owner" } else { "account" },
        actor_accid: id.accid(),
        actor_login: id.login().map(String::from),
        actor_role: id.role.as_str(),
        ip: Some(ip.to_string()),
        action,
        target_kind: Some(kind),
        target_id: target,
        target_name: Some(name.to_string()),
        detail: Some(detail.to_string()),
        result: "ok",
        ..Default::default()
    }
}

/// Who edited, as shown to the owner: the account's login, or "Owner link".
fn editor(id: &Identity) -> String {
    id.login().unwrap_or("Owner link").to_string()
}

fn admin_row(p: &NewsPost) -> Value {
    json!({
        "id": p.id, "title": p.title, "author": p.author, "published": p.published, "pinned": p.pinned,
        "created": p.created, "updated": p.updated, "updated_by": p.updated_by,
    })
}

async fn list(State(state): State<Shared>, Owner(_): Owner) -> ApiResult<Json<Value>> {
    let posts = state.store.news_all().await?;
    Ok(Json(json!({ "items": posts.iter().map(admin_row).collect::<Vec<_>>(), "max_title": MAX_TITLE, "max_body": MAX_BODY, "max_author": MAX_AUTHOR })))
}

async fn read(State(state): State<Shared>, Owner(_): Owner, Path(id): Path<i64>) -> ApiResult<Json<NewsPost>> {
    Ok(Json(state.store.news_post(id).await?.ok_or_else(ApiError::not_found)?))
}

#[derive(Deserialize)]
struct PostBody {
    title: String,
    body: String,
    /// The name shown under the post; "Staff" when left empty.
    author: Option<String>,
    #[serde(default)]
    pinned: bool,
    /// `true` publishes, `false` makes it a draft; absent keeps what it is (new posts are drafts).
    publish: Option<bool>,
}

fn clean(b: &PostBody) -> ApiResult<PostFields> {
    let title = b.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("Give the post a title."));
    }
    if title.chars().count() > MAX_TITLE || title.chars().any(|c| c.is_control()) {
        return Err(ApiError::bad_request("The title must be one line of at most 120 characters."));
    }
    if b.body.len() > MAX_BODY {
        return Err(ApiError::bad_request("The post is too long: 20,000 characters at most."));
    }
    if b.body.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(ApiError::bad_request("The text contains control characters."));
    }
    let author = b.author.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or(DEFAULT_AUTHOR);
    if author.chars().count() > MAX_AUTHOR || author.chars().any(|c| c.is_control()) {
        return Err(ApiError::bad_request("The author name must be at most 40 characters on one line."));
    }
    if b.publish == Some(true) && b.body.trim().is_empty() {
        return Err(ApiError::bad_request("Write something before publishing."));
    }
    Ok(PostFields { title: title.to_string(), body: b.body.replace("\r\n", "\n"), author: author.to_string(), pinned: b.pinned })
}

async fn create(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Json(b): Json<PostBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    let f = clean(&b)?;
    let publish = b.publish == Some(true);
    let new_id = state.store.news_create(&f, publish, &editor(&id), state.now()).await?;
    state.news_cache.clear();
    let action = if publish { "news_publish" } else { "news_create" };
    let detail = json!({ "after": { "title": f.title, "body_size": f.body.len(), "published": publish, "pinned": f.pinned } });
    record(&state, act(&id, &ip, action, "news", Some(new_id), &f.title, detail)).await;
    Ok((StatusCode::CREATED, Json(json!({ "id": new_id }))))
}

async fn update(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Path(post_id): Path<i64>, Json(b): Json<PostBody>) -> ApiResult<Json<Value>> {
    let before = state.store.news_post(post_id).await?.ok_or_else(ApiError::not_found)?;
    let f = clean(&b)?;
    if b.publish == Some(true) && before.published.is_none() && b.body.trim().is_empty() {
        return Err(ApiError::bad_request("Write something before publishing."));
    }
    if !state.store.news_update(post_id, &f, b.publish, &editor(&id), state.now()).await? {
        return Err(ApiError::not_found());
    }
    state.news_cache.clear();
    let was_out = before.published.is_some();
    let action = match b.publish {
        Some(true) if !was_out => "news_publish",
        Some(false) if was_out => "news_unpublish",
        _ if f.pinned != before.pinned && f.title == before.title && f.body == before.body && f.author == before.author => {
            if f.pinned { "news_pin" } else { "news_unpin" }
        }
        _ => "news_edit",
    };
    let detail = json!({
        "before": { "title": before.title, "body_size": before.body.len(), "published": was_out, "pinned": before.pinned },
        "after": { "title": f.title, "body_size": f.body.len(), "published": b.publish.unwrap_or(was_out), "pinned": f.pinned },
    });
    record(&state, act(&id, &ip, action, "news", Some(post_id), &f.title, detail)).await;
    Ok(Json(json!({ "id": post_id })))
}

async fn remove(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Path(post_id): Path<i64>) -> ApiResult<Json<Value>> {
    let before = state.store.news_post(post_id).await?.ok_or_else(ApiError::not_found)?;
    if !state.store.news_delete(post_id).await? {
        return Err(ApiError::not_found());
    }
    state.news_cache.clear();
    let detail = json!({ "before": { "title": before.title, "body_size": before.body.len(), "published": before.published.is_some(), "pinned": before.pinned } });
    record(&state, act(&id, &ip, "news_delete", "news", Some(post_id), &before.title, detail)).await;
    Ok(Json(json!({ "deleted": post_id })))
}

// ---- site content ----

fn block_json(b: &content::Block, stored: &[crate::store::ContentRow], state: &Shared) -> Value {
    let row = stored.iter().find(|r| r.key == b.key);
    json!({
        "key": b.key, "label": b.label, "hint": b.hint, "max": b.max, "plain": b.plain,
        "body": row.map(|r| r.body.clone()).unwrap_or_else(|| content::default_body(b.key, &state.cfg)),
        "default": content::default_body(b.key, &state.cfg),
        "is_default": row.is_none(),
        "updated": row.map(|r| r.updated), "updated_by": row.and_then(|r| r.updated_by.clone()),
    })
}

async fn blocks(State(state): State<Shared>, Owner(_): Owner) -> ApiResult<Json<Value>> {
    let stored = state.store.content_all().await?;
    Ok(Json(json!({ "blocks": content::BLOCKS.iter().map(|b| block_json(b, &stored, &state)).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
struct BlockBody {
    body: String,
}

async fn save_block(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Path(key): Path<String>, Json(b): Json<BlockBody>) -> ApiResult<Json<Value>> {
    let block = content::block(&key).ok_or_else(ApiError::not_found)?;
    let text = b.body.replace("\r\n", "\n");
    let text = if block.plain { text.trim().to_string() } else { text };
    content::check(block, &text).map_err(|m| ApiError::bad_request(&m))?;
    let stored = state.store.content_all().await?;
    let (before, _) = content::effective(block.key, &stored, &state.cfg);
    state.store.content_set(block.key, &text, &editor(&id), state.now()).await?;
    state.news_cache.clear();
    let detail = if block.plain {
        json!({ "before": before, "after": text })
    } else {
        json!({ "before_size": before.len(), "after_size": text.len() })
    };
    record(&state, act(&id, &ip, "content_edit", "content", None, block.label, detail)).await;
    Ok(Json(json!({ "saved": block.key })))
}

async fn reset_block(State(state): State<Shared>, Owner(id): Owner, ClientIp(ip): ClientIp, Path(key): Path<String>) -> ApiResult<Json<Value>> {
    let block = content::block(&key).ok_or_else(ApiError::not_found)?;
    let stored = state.store.content_all().await?;
    let (before, _) = content::effective(block.key, &stored, &state.cfg);
    let had = state.store.content_reset(block.key).await?;
    state.news_cache.clear();
    if had {
        let detail = json!({ "before_size": before.len(), "after_size": content::default_body(block.key, &state.cfg).len(), "reset": true });
        record(&state, act(&id, &ip, "content_reset", "content", None, block.label, detail)).await;
    }
    Ok(Json(json!({ "reset": block.key })))
}

// ---- preview ----

#[derive(Deserialize)]
struct PreviewBody {
    markdown: String,
}

/// The same renderer visitors get, for Write/Preview. Owner only, like the editors it serves.
async fn preview(State(_state): State<Shared>, Owner(_): Owner, Json(b): Json<PreviewBody>) -> ApiResult<Json<Value>> {
    if b.markdown.len() > MAX_BODY {
        return Err(ApiError::bad_request("The text is too long."));
    }
    Ok(Json(json!({ "html": markdown::render(&b.markdown) })))
}
