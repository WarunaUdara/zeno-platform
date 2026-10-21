mock_provider "google" {}

run "production_release_requires_enabled_workloads_and_public_web" {
  command = plan

  variables {
    project_id         = "zeno-audit-test"
    production_release = true
  }

  expect_failures = [var.production_release]
}

run "production_release_requires_neon_auth_configuration" {
  command = plan

  variables {
    project_id                            = "zeno-auth-gate-test"
    deploy_workloads                      = true
    publish_web                           = true
    production_release                    = true
    public_domain                         = "events.example.com"
    public_reservation_rate_limit_preview = false
    monitoring_notification_channels      = ["projects/zeno-auth-gate-test/notificationChannels/123"]
    api_image                             = "asia-southeast1-docker.pkg.dev/zeno-auth-gate-test/images/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    web_image                             = "asia-southeast1-docker.pkg.dev/zeno-auth-gate-test/images/web@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }

  expect_failures = [var.production_release]
}

run "neon_auth_issuer_and_jwks_must_be_configured_together" {
  command = plan

  variables {
    project_id         = "zeno-auth-pair-test"
    neon_auth_base_url = "https://auth.example.com"
  }

  expect_failures = [var.neon_auth_base_url]
}

run "web_auth_cookie_secret_version_must_be_pinned_numeric" {
  command = plan

  variables {
    project_id                     = "zeno-auth-secret-test"
    web_auth_cookie_secret_version = "latest"
  }

  expect_failures = [var.web_auth_cookie_secret_version]
}

run "production_release_wires_configured_neon_auth" {
  command = plan

  variables {
    project_id                                   = "zeno-auth-ready-test"
    environment                                  = "test"
    deploy_workloads                             = true
    publish_web                                  = true
    production_release                           = true
    public_domain                                = "events.example.com"
    public_reservation_rate_limit_preview        = false
    monitoring_notification_channels             = ["projects/zeno-auth-ready-test/notificationChannels/123"]
    neon_auth_base_url                           = "https://auth.example.com"
    neon_auth_jwks_url                           = "https://auth.example.com/.well-known/jwks.json"
    web_auth_cookie_secret_version               = "1"
    neon_object_storage_endpoint_url             = "https://storage.example.com"
    neon_object_storage_region                   = "ap-southeast-1"
    web_storage_access_key_secret_version        = "2"
    web_storage_secret_access_key_secret_version = "3"
    api_image                                    = "asia-southeast1-docker.pkg.dev/zeno-auth-ready-test/images/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    web_image                                    = "asia-southeast1-docker.pkg.dev/zeno-auth-ready-test/images/web@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }

  assert {
    condition = anytrue([
      for env in google_cloud_run_v2_service.api[0].template[0].containers[0].env :
      env.name == "NEON_AUTH_BASE_URL" && env.value == "https://auth.example.com"
      ]) && anytrue([
      for env in google_cloud_run_v2_service.api[0].template[0].containers[0].env :
      env.name == "NEON_AUTH_JWKS_URL" && env.value == "https://auth.example.com/.well-known/jwks.json"
      ]) && !anytrue([
      for env in google_cloud_run_v2_service.api[0].template[0].containers[0].env :
      env.name == "NEON_AUTH_COOKIE_SECRET"
    ])
    error_message = "The API runtime must receive issuer/JWKS URLs and must not receive the web cookie secret."
  }

  assert {
    condition = anytrue([
      for env in google_cloud_run_v2_service.web[0].template[0].containers[0].env :
      env.name == "NEON_AUTH_BASE_URL" && env.value == "https://auth.example.com"
      ]) && anytrue([
      for env in google_cloud_run_v2_service.web[0].template[0].containers[0].env :
      env.name == "NEON_AUTH_COOKIE_SECRET" && env.value_source[0].secret_key_ref[0].version == "1"
      ]) && anytrue([
      for env in google_cloud_run_v2_service.web[0].template[0].containers[0].env :
      env.name == "AWS_ACCESS_KEY_ID" && env.value_source[0].secret_key_ref[0].version == "2"
      ]) && anytrue([
      for env in google_cloud_run_v2_service.web[0].template[0].containers[0].env :
      env.name == "AWS_SECRET_ACCESS_KEY" && env.value_source[0].secret_key_ref[0].version == "3"
      ]) && anytrue([
      for env in google_cloud_run_v2_service.web[0].template[0].containers[0].env :
      env.name == "AWS_ENDPOINT_URL_S3" && env.value == "https://storage.example.com"
      ]) && anytrue([
      for env in google_cloud_run_v2_service.web[0].template[0].containers[0].env :
      env.name == "AWS_REGION" && env.value == "ap-southeast-1"
      ]) && !anytrue([
      for env in google_cloud_run_v2_service.api[0].template[0].containers[0].env :
      contains(["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_ENDPOINT_URL_S3", "AWS_REGION"], env.name)
    ])
    error_message = "Only the web runtime must receive pinned object-storage credentials and branch configuration."
  }
}

run "publish_web_and_default_uri_are_mutually_exclusive" {
  command = plan

  variables {
    project_id               = "zeno-demo-uri-conflict-test"
    deploy_workloads         = true
    publish_web              = true
    allow_public_default_uri = true
    public_domain            = "events.example.com"
    api_image                = "asia-southeast1-docker.pkg.dev/zeno-demo-uri-conflict-test/images/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    web_image                = "asia-southeast1-docker.pkg.dev/zeno-demo-uri-conflict-test/images/web@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }

  expect_failures = [var.allow_public_default_uri]
}

run "allow_public_default_uri_exposes_the_run_app_url" {
  command = plan

  variables {
    project_id               = "zeno-demo-uri-test"
    environment              = "test"
    deploy_workloads         = true
    allow_public_default_uri = true
    api_image                = "asia-southeast1-docker.pkg.dev/zeno-demo-uri-test/images/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    web_image                = "asia-southeast1-docker.pkg.dev/zeno-demo-uri-test/images/web@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }

  # Granting allUsers run.invoker is what makes the default URL usable at all.
  # Without it the service answers 403 to every request even though the URL exists.
  assert {
    condition     = google_cloud_run_v2_service_iam_member.web_public_invoker[0].member == "allUsers"
    error_message = "allow_public_default_uri must grant the web service public invocation."
  }

  assert {
    condition = (
      google_cloud_run_v2_service.web[0].default_uri_disabled == false &&
      google_cloud_run_v2_service.web[0].ingress == "INGRESS_TRAFFIC_ALL"
    )
    error_message = "allow_public_default_uri must leave the default run.app URI enabled and the ingress open."
  }

  # The API must stay private; the web tier reaches it with a Cloud Run identity token.
  assert {
    condition     = length(google_cloud_run_v2_service_iam_member.api_invoker) == 1 && google_cloud_run_v2_service_iam_member.api_invoker[0].member != "allUsers"
    error_message = "Exposing the frontend must not make the API publicly invokable."
  }
}
