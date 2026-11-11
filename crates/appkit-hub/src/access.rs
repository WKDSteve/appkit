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
    if uri.path().starts_with("/assets/")
        || st.guard_rules.is_installable(uri.path())
        || st.ui.has_index()
    {
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

// ── device keys ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DevicePairRequest {
    pub code: String,
    pub name: String,
    /// The public key, SEC1 uncompressed P-256, lower-case hex.
    pub key: String,
    /// The key that confirms a trigger, kept by the device behind its biometrics or lock; as `key`. Optional: a
    /// device without it never triggers.
    #[serde(default)]
    pub confirm_key: Option<String>,
}

#[derive(Deserialize)]
pub struct DeviceChallengeRequest {
    pub device: String,
}

#[derive(Deserialize)]
pub struct DeviceSessionRequest {
    pub device: String,
    pub challenge: String,
    /// P1363 (r and s), lower-case hex, as WebCrypto signs.
    pub signature: String,
}

#[derive(Deserialize)]
pub struct DeviceRevokeRequest {
    pub device: String,
}

#[derive(Deserialize)]
pub struct DeviceLevelRequest {
    pub device: String,
    pub level: String,
}

fn no_devices() -> Response {
    error_body(StatusCode::NOT_FOUND, "not_found", "no such route")
}

fn session_answer(
    made: Result<Secret, appkit_core::authn::RandomError>,
    device: Option<(&str, &str)>,
) -> Response {
    match made {
        Ok(token) => match device {
            Some((id, hub)) => {
                axum::Json(json!({ "device": id, "hub": hub, "token": token.to_hex() }))
                    .into_response()
            }
            None => axum::Json(json!({ "token": token.to_hex() })).into_response(),
        },
        Err(_) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "no session could be made",
        ),
    }
}

/// Pair a device with a one-time code and its public key; the answer is its id and a first session.
pub async fn device_pair<R: Runtime>(State(st): State<Arc<Core<R>>>, body: Bytes) -> Response {
    let Some(devices) = &st.devices else {
        return no_devices();
    };
    if !st.exchange_limit.allow() {
        return error_body(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many attempts",
        );
    }
    let Ok(request) = serde_json::from_slice::<DevicePairRequest>(&body) else {
        return error_body(StatusCode::BAD_REQUEST, "bad_body", "invalid request body");
    };
    // The key and the name are checked before the code is spent: a mistake of the page does not cost the code.
    if crate::devices::check_pairing(&request.name, &request.key, request.confirm_key.as_deref())
        .is_err()
    {
        return error_body(
            StatusCode::BAD_REQUEST,
            "bad_device",
            "the name or the key is not valid",
        );
    }
    if !st.pairing.redeem(&request.code) {
        return error_body(
            StatusCode::UNAUTHORIZED,
            "bad_code",
            "the code is not valid",
        );
    }
    match devices.pair(
        &request.name,
        &request.key,
        request.confirm_key.as_deref(),
        st.guard_rules.device_default_level(),
    ) {
        Ok(id) => session_answer(
            st.sessions.create_for_device(&id),
            Some((&id, devices.hub_id())),
        ),
        Err(crate::devices::PairError::Full) => error_body(
            StatusCode::CONFLICT,
            "devices_full",
            "the list of devices is full: revoke one on the PC",
        ),
        Err(crate::devices::PairError::Known) => error_body(
            StatusCode::CONFLICT,
            "device_known",
            "this key is paired already",
        ),
        Err(crate::devices::PairError::BadName | crate::devices::PairError::BadKey) => error_body(
            StatusCode::BAD_REQUEST,
            "bad_device",
            "the name or the key is not valid",
        ),
        Err(crate::devices::PairError::Io | crate::devices::PairError::Random) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the device could not be kept",
        ),
    }
}

/// A challenge for a device to sign. Every id gets one: the answer does not tell which devices exist.
pub async fn device_challenge<R: Runtime>(State(st): State<Arc<Core<R>>>, body: Bytes) -> Response {
    let Some(devices) = &st.devices else {
        return no_devices();
    };
    if !st.exchange_limit.allow() {
        return error_body(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many attempts",
        );
    }
    let Ok(request) = serde_json::from_slice::<DeviceChallengeRequest>(&body) else {
        return error_body(StatusCode::BAD_REQUEST, "bad_body", "invalid request body");
    };
    match devices.challenge(&request.device) {
        Ok(challenge) => {
            axum::Json(json!({ "challenge": challenge, "hub": devices.hub_id() })).into_response()
        }
        Err(_) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "no challenge could be made",
        ),
    }
}

/// A session for a device that signed its challenge, for the origin the request comes from (the guard has
/// checked it is this hub's).
pub async fn device_session<R: Runtime>(
    State(st): State<Arc<Core<R>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(devices) = &st.devices else {
        return no_devices();
    };
    if !st.exchange_limit.allow() {
        return error_body(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many attempts",
        );
    }
    let Ok(request) = serde_json::from_slice::<DeviceSessionRequest>(&body) else {
        return error_body(StatusCode::BAD_REQUEST, "bad_body", "invalid request body");
    };
    // The guard requires an Origin on this route and has checked it; it is signed with the challenge, so a
    // signature made for another origin opens nothing here.
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return error_body(
            StatusCode::FORBIDDEN,
            "origin_required",
            "an Origin is required",
        );
    };
    // The same answer for an unknown device, a wrong signature and a spent or old challenge.
    if !devices.login(
        &request.device,
        &request.challenge,
        &request.signature,
        origin,
    ) {
        return error_body(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "the device could not be verified",
        );
    }
    session_answer(
        st.sessions.create_for_device(&request.device),
        Some((&request.device, devices.hub_id())),
    )
}

/// The paired devices, for the PC (control secret).
pub async fn control_devices<R: Runtime>(State(st): State<Arc<Core<R>>>) -> Response {
    let Some(devices) = &st.devices else {
        return no_devices();
    };
    axum::Json(json!({ "devices": devices.list() })).into_response()
}

/// Set the level of a device, for the PC (control secret). Its open sessions keep going at the new level: the guard
/// asks the level on every request.
pub async fn control_level<R: Runtime>(State(st): State<Arc<Core<R>>>, body: Bytes) -> Response {
    let Some(devices) = &st.devices else {
        return no_devices();
    };
    let Ok(request) = serde_json::from_slice::<DeviceLevelRequest>(&body) else {
        return error_body(StatusCode::BAD_REQUEST, "bad_body", "invalid request body");
    };
    let Some(level) = crate::devices::Level::parse(&request.level) else {
        return error_body(
            StatusCode::BAD_REQUEST,
            "bad_level",
            "read, create, change or trigger",
        );
    };
    match devices.set_level(&request.device, level) {
        Ok(found) => axum::Json(json!({ "set": found })).into_response(),
        Err(_) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the list of devices could not be written; the level is as it was",
        ),
    }
}

/// Revoke a device, for the PC (control secret): it is forgotten and its sessions are closed.
pub async fn control_revoke<R: Runtime>(State(st): State<Arc<Core<R>>>, body: Bytes) -> Response {
    let Some(devices) = &st.devices else {
        return no_devices();
    };
    let Ok(request) = serde_json::from_slice::<DeviceRevokeRequest>(&body) else {
        return error_body(StatusCode::BAD_REQUEST, "bad_body", "invalid request body");
    };
    match devices.revoke(&request.device) {
        Ok(found) => {
            // Closed also when the file could not be written below: a revoke that half failed still ends the sessions.
            st.sessions.close_device(&request.device);
            axum::Json(json!({ "revoked": found })).into_response()
        }
        Err(_) => {
            st.sessions.close_device(&request.device);
            error_body(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "the list of devices could not be written; the device's sessions are closed",
            )
        }
    }
}
