//! The routes of the page and of the pairing: the shell, `hello`, `exchange` and `control_pair`.

use crate::*;

// ── routes ──────────────────────────────────────────────────────────────────

pub async fn shell<R: Runtime>(State(st): State<Arc<Core<R>>>, uri: axum::http::Uri) -> Response {
    if let Some(file) = st.ui.page(uri.path()) {
        return (
            [(header::CONTENT_TYPE, file.content_type)],
            Body::from(file.bytes),
        )
            .into_response();
    }
    // Under /assets/ only what the build holds; the page itself is the
    // placeholder until an interface is there.
    if uri.path().starts_with("/assets/") || st.ui.has_index() {
        return error_body(StatusCode::NOT_FOUND, "not_found", "no such file");
    }
    let app = st.app;
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{app}</title></head><body><p>{app} hub is running. The interface is not built yet.</p></body></html>"
    );
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Body::from(html),
    )
        .into_response()
}

pub async fn hello<R: Runtime>(State(st): State<Arc<Core<R>>>) -> Response {
    let rt = st.rt();
    // Version and abilities only: no task, no area, no path, no vault name.
    let setup = rt.setup_required();
    let body = json!({
        "schema": 1,
        "kind": "hub.hello",
        "app": st.app,
        "version": st.version,
        // The short commit id of a rolling build, else null; added to schema 1 like the keys below.
        "build": st.build_id.and_then(crate::hub::clean_build_id),
        "instance": st.instance,
        // Added to schema 1: a reader that does not know them ignores them.
        "setup_required": setup.is_some(),
        "setup_reason": setup,
        "engine": {
            "ready": rt.status().engine_ready,
            "problem": rt.status().problem,
            "capabilities": rt.status().capabilities,
        },
        "capabilities": ["pairing"],
    });
    axum::Json(body).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeRequest {
    pub code: String,
}

pub async fn exchange<R: Runtime>(State(st): State<Arc<Core<R>>>, body: Bytes) -> Response {
    if !st.exchange_limit.allow() {
        return error_body(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many attempts",
        );
    }
    let Ok(request) = serde_json::from_slice::<ExchangeRequest>(&body) else {
        return error_body(StatusCode::BAD_REQUEST, "bad_body", "invalid request body");
    };
    // The same answer for an unknown, a spent and an expired code.
    if !st.pairing.redeem(&request.code) {
        return error_body(
            StatusCode::UNAUTHORIZED,
            "bad_code",
            "the code is not valid",
        );
    }
    match st.sessions.create() {
        Ok(token) => axum::Json(json!({ "token": token.to_hex() })).into_response(),
        Err(_) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "no session could be made",
        ),
    }
}

pub async fn control_pair<R: Runtime>(State(st): State<Arc<Core<R>>>) -> Response {
    match st.pairing.mint() {
        Ok(code) => {
            debug_assert_eq!(code.to_hex().len(), CODE_BYTES * 2);
            axum::Json(json!({ "code": code.to_hex() })).into_response()
        }
        Err(_) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "no code could be made",
        ),
    }
}
