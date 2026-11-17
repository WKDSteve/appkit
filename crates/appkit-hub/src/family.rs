//! `GET /api/family`: which apps of the family are installed and which run, for the page of the app
//! (a button that hands something to another app is shown only while that app is there).
//!
//! The answer is [`appkit_core::family::family_apps`]: a few file reads, no network, no secret, no
//! path. Every request reads the files anew; the page asks at its start and again when its window
//! gets the focus (or before a hand-over), since port and instance change with every start of the
//! other app. A route of the core: it needs a session, and a paired device does not reach it.

use std::path::PathBuf;
use std::sync::Arc;

use appkit_core::family::{self, FamilyApp};
use axum::extract::{RawQuery, State};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::env::Env;
use crate::hub::{Core, Runtime};
use crate::no_query;

/// The path of the route.
pub const FAMILY_PATH: &str = "/api/family";

/// The body of the answer: `{ "schema": 1, "kind": "hub.family", "apps": [...] }`.
#[must_use]
pub fn answer(apps: &[FamilyApp]) -> serde_json::Value {
    json!({ "schema": 1, "kind": "hub.family", "apps": apps })
}

/// The apps of the family as the environment `env` finds them.
#[must_use]
pub fn apps_in(env: &Env) -> Vec<FamilyApp> {
    let vars = |name: &str| -> Option<PathBuf> { env.path(name) };
    family::family_apps(&vars, &[])
}

/// `GET /api/family`.
pub async fn family_list<R: Runtime>(
    State(_st): State<Arc<Core<R>>>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Some(response) = no_query(query.as_deref()) {
        return response;
    }
    let apps = apps_in(&Env::from_process());
    axum::Json(answer(&apps)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::family::{write_manifest, Manifest, MANIFEST_SCHEMA};
    use appkit_core::registry::{write_record, HubRecord};
    use appkit_scratch::Scratch;

    #[test]
    fn the_answer_lists_installed_and_running_apps_without_a_path() {
        let root = Scratch::new("hub-family");
        let program = root.join("ai-desktop.exe");
        std::fs::create_dir_all(&*root).unwrap();
        std::fs::write(&program, b"x").unwrap();
        write_manifest(
            &root.join("family"),
            &Manifest {
                schema: MANIFEST_SCHEMA.to_owned(),
                app: "ai-desktop".to_owned(),
                version: Some("0.1.0".to_owned()),
                program,
                env_prefix: "AI_DESKTOP".to_owned(),
            },
        )
        .unwrap();
        write_record(
            &root.join("ai"),
            &HubRecord {
                app: "ai-desktop".to_owned(),
                port: 8080,
                pid: std::process::id(),
                version: "0.1.1".to_owned(),
                instance: "ef".repeat(16),
            },
        )
        .unwrap();
        let env = Env::from_pairs([
            (family::FAMILY_DIR_VAR, root.join("family")),
            ("AI_DESKTOP_CONFIG_DIR", root.join("ai")),
        ]);
        let doc = answer(&apps_in(&env));
        assert_eq!(
            doc,
            json!({"schema": 1, "kind": "hub.family", "apps": [{
                "app": "ai-desktop", "installed": true, "running": true, "port": 8080,
                "version": "0.1.1", "instance": "ef".repeat(16),
            }]})
        );
        assert!(!doc.to_string().contains(".exe"));
    }

    #[test]
    fn no_family_folder_is_an_empty_list() {
        let env = Env::from_pairs::<_, &str, &str>([]);
        assert_eq!(answer(&apps_in(&env))["apps"], json!([]));
    }
}
