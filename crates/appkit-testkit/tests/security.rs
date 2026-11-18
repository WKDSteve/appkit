//! The security matrix of the guard (concept 5.5), run for `Demo`: a minimal app on `appkit-hub`
//! alone, with its own identity and namespace. That it passes shows that the guard depends on no
//! app. Every app that has a hub runs the same matrix for itself.

mod demo {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::Demo);
}

mod demo_wide {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoWide);
}

#[tokio::test]
async fn an_app_without_an_extra_source_keeps_connect_src_self() {
    appkit_testkit::security::the_page_may_connect_to(&appkit_testkit::subjects::Demo, None).await;
}

#[tokio::test]
async fn an_app_that_names_one_https_host_gets_exactly_that_one() {
    appkit_testkit::security::the_page_may_connect_to(
        &appkit_testkit::subjects::DemoWide,
        Some(appkit_testkit::subjects::WIDE_ORIGIN),
    )
    .await;
}

mod demo_installable {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoInstallable);
}

#[tokio::test]
async fn an_installable_page_has_its_own_policy_and_nothing_more() {
    appkit_testkit::security::the_installable_page_has_its_own_policy_and_nothing_more(
        &appkit_testkit::subjects::DemoInstallable,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
        appkit_testkit::subjects::INSTALLABLE_WORKER_POLICY,
    )
    .await;
}

mod demo_installable_recording {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoInstallableRecording);
}

#[tokio::test]
async fn an_installable_page_that_records_differs_in_the_microphone_and_media_only() {
    appkit_testkit::security::the_installable_page_has_its_own_policy_and_nothing_more(
        &appkit_testkit::subjects::DemoInstallableRecording,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
        appkit_testkit::subjects::INSTALLABLE_WORKER_POLICY,
    )
    .await;
    appkit_testkit::security::the_installable_page_may_record_and_nothing_else(
        &appkit_testkit::subjects::DemoInstallableRecording,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
    )
    .await;
}

#[tokio::test]
#[should_panic(expected = "/m/: camera=(), microphone=()")]
async fn an_installable_page_that_was_not_let_record_does_not() {
    appkit_testkit::security::the_installable_page_may_record_and_nothing_else(
        &appkit_testkit::subjects::DemoInstallable,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
    )
    .await;
}

mod demo_capturing {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoCapturing);
}

#[tokio::test]
async fn a_page_that_was_let_capture_differs_in_camera_and_microphone_only() {
    appkit_testkit::security::the_page_may_capture(
        &appkit_testkit::subjects::DemoCapturing,
        true,
        Some(appkit_testkit::subjects::INSTALLABLE_PREFIX),
    )
    .await;
    // The installable page of that app is the one of an app without the switch, recording included.
    appkit_testkit::security::the_installable_page_has_its_own_policy_and_nothing_more(
        &appkit_testkit::subjects::DemoCapturing,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
        appkit_testkit::subjects::INSTALLABLE_WORKER_POLICY,
    )
    .await;
    appkit_testkit::security::the_installable_page_may_record_and_nothing_else(
        &appkit_testkit::subjects::DemoCapturing,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
    )
    .await;
}

#[tokio::test]
async fn a_page_that_was_not_let_capture_keeps_camera_and_microphone_off() {
    appkit_testkit::security::the_page_may_capture(&appkit_testkit::subjects::Demo, false, None)
        .await;
    appkit_testkit::security::the_page_may_capture(
        &appkit_testkit::subjects::DemoInstallableRecording,
        false,
        Some(appkit_testkit::subjects::INSTALLABLE_PREFIX),
    )
    .await;
}

#[tokio::test]
#[should_panic(expected = "/: ")]
async fn a_page_that_was_not_let_capture_does_not() {
    appkit_testkit::security::the_page_may_capture(
        &appkit_testkit::subjects::DemoInstallableRecording,
        true,
        Some(appkit_testkit::subjects::INSTALLABLE_PREFIX),
    )
    .await;
}

/// An app that sets `GuardRules::with_page_capture` (it is `DemoCapturing`) but does not say so: the subject keeps the
/// default of `Subject::page_may_capture`.
struct CapturingUnsaid;

impl appkit_testkit::subject::Subject for CapturingUnsaid {
    fn identity(&self) -> &'static appkit_core::identity::AppIdentity {
        appkit_testkit::subject::Subject::identity(&appkit_testkit::subjects::DemoCapturing)
    }

    fn start(
        &self,
        cfg: appkit_testkit::subject::StartConfig,
    ) -> appkit_hub::BoxFuture<'_, std::io::Result<appkit_hub::RunningServer>> {
        appkit_testkit::subject::Subject::start(&appkit_testkit::subjects::DemoCapturing, cfg)
    }

    fn data_routes(&self) -> Vec<&'static str> {
        appkit_testkit::subject::Subject::data_routes(&appkit_testkit::subjects::DemoCapturing)
    }
}

#[tokio::test]
#[should_panic(expected = "/: ")]
async fn the_matrix_is_red_for_an_app_that_captures_without_saying_so() {
    appkit_testkit::security::the_page_captures_as_the_subject_says(&CapturingUnsaid).await;
}

#[tokio::test]
#[should_panic(expected = "the permissions of the desktop page")]
async fn the_installable_check_takes_no_capturing_desktop_page_that_was_not_said() {
    appkit_testkit::security::the_installable_page_has_its_own_policy_and_nothing_more(
        &CapturingUnsaid,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
        appkit_testkit::subjects::INSTALLABLE_WORKER_POLICY,
    )
    .await;
}

#[tokio::test]
#[should_panic(expected = "/: the permissions of the desktop page")]
async fn the_record_check_takes_no_capturing_desktop_page_that_was_not_said() {
    appkit_testkit::security::the_installable_page_may_record_and_nothing_else(
        &CapturingUnsaid,
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
    )
    .await;
}

mod demo_devices {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoDevices::new());
}

mod demo_devices_capturing {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoDevices::capturing());
}

/// Decided 2026-11-18: the camera and the microphone stay allowed when the shell is served through the tailnet name;
/// the browser of each device asks its person.
#[tokio::test]
async fn a_page_that_was_let_capture_may_capture_through_the_tailnet_too() {
    appkit_testkit::security::the_page_may_capture(
        &appkit_testkit::subjects::DemoDevices::capturing(),
        true,
        Some(appkit_testkit::subjects::INSTALLABLE_PREFIX),
    )
    .await;
    appkit_testkit::security::the_page_may_capture(
        &appkit_testkit::subjects::DemoDevices::new(),
        false,
        Some(appkit_testkit::subjects::INSTALLABLE_PREFIX),
    )
    .await;
}

#[tokio::test]
async fn a_device_pairs_once_and_then_logs_in_with_its_key() {
    appkit_testkit::security::a_device_pairs_once_and_then_logs_in_with_its_key(
        &appkit_testkit::subjects::DemoDevices::new(),
    )
    .await;
}

#[tokio::test]
async fn device_pairing_checks_before_it_spends_the_code() {
    appkit_testkit::security::device_pairing_checks_before_it_spends_the_code(
        &appkit_testkit::subjects::DemoDevices::new(),
    )
    .await;
}

#[tokio::test]
async fn the_pc_revokes_a_device_and_its_sessions_end() {
    appkit_testkit::security::the_pc_revokes_a_device_and_its_sessions_end(
        &appkit_testkit::subjects::DemoDevices::new(),
    )
    .await;
}

#[tokio::test]
async fn another_app_may_open_the_installable_page_and_nothing_else() {
    appkit_testkit::security::another_app_may_open_the_installable_page_and_nothing_else(
        &appkit_testkit::subjects::DemoDevices::new(),
        appkit_testkit::subjects::INSTALLABLE_PREFIX,
    )
    .await;
}

#[tokio::test]
async fn without_devices_there_are_no_device_routes() {
    appkit_testkit::security::without_devices_there_are_no_device_routes(
        &appkit_testkit::subjects::Demo,
    )
    .await;
}

#[tokio::test]
async fn the_tailnet_takes_devices_only() {
    appkit_testkit::security::the_tailnet_takes_devices_only(
        &appkit_testkit::subjects::DemoDevices::new(),
        appkit_testkit::subjects::TAILNET_NAME,
    )
    .await;
}

#[tokio::test]
async fn a_device_reaches_only_released_routes_and_no_private_data() {
    appkit_testkit::security::a_device_reaches_only_released_routes_and_no_private_data(
        &appkit_testkit::subjects::DemoDevices::new(),
        &["items", "item", "classified"],
        "pc-only",
        "classified",
        &appkit_testkit::subjects::CLASSIFIED,
    )
    .await;
}

#[tokio::test]
async fn a_device_does_only_what_its_level_allows_and_triggers_only_confirmed() {
    appkit_testkit::security::a_device_does_only_what_its_level_allows_and_triggers_only_confirmed(
        &appkit_testkit::subjects::DemoDevices::new(),
        "items",
        "ops",
        "status",
        "run",
    )
    .await;
}

#[tokio::test]
async fn devices_subscribe_to_push_and_nothing_else_can() {
    let subject = appkit_testkit::subjects::DemoDevices::new();
    let push = std::sync::Arc::clone(subject.push());
    appkit_testkit::security::devices_subscribe_to_push_and_nothing_else_can(&subject, &|id| {
        !push.kinds_of(id).is_empty()
    })
    .await;
}

#[tokio::test]
async fn a_push_names_only_a_kind_and_a_released_id() {
    use appkit_hub::DataClass;
    let subject = appkit_testkit::subjects::DemoDevices::new();
    let key = p256::SecretKey::from_slice(&[5; 32]).unwrap();
    use p256::elliptic_curve::sec1::ToSec1Point;
    subject
        .push()
        .subscribe(
            "ab".repeat(16).as_str(),
            appkit_hub::push::Subscription {
                endpoint: "https://push.example.org/x".to_owned(),
                p256dh: appkit_hub::push::b64url(key.public_key().to_sec1_point(false).as_bytes()),
                auth: appkit_hub::push::b64url(&[1; 16]),
                kinds: vec!["ci_red".to_owned()],
            },
        )
        .unwrap();
    let sent = subject
        .push()
        .notify("ci_red", Some(("secret-area", DataClass::Private)), true)
        .await;
    assert_eq!(sent.delivered, 1);
    let taken = subject.sent().taken();
    assert_eq!(taken.len(), 1);
    // Encrypted, and the private id was never in the text.
    assert!(!String::from_utf8_lossy(&taken[0].body).contains("secret-area"));
    assert_eq!(
        appkit_hub::push::Push::payload("ci_red", Some(("secret-area", DataClass::Private))),
        r#"{"id":null,"kind":"ci_red"}"#
    );
}
