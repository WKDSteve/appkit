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
