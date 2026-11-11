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

mod demo_devices {
    appkit_testkit::security_matrix!(appkit_testkit::subjects::DemoDevices::new());
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
