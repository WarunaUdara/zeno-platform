resource "google_cloud_run_v2_service" "api" {
  count = var.deploy_workloads ? 1 : 0

  project             = var.project_id
  name                = "zeno-${var.environment}-api"
  location            = var.region
  ingress             = "INGRESS_TRAFFIC_ALL"
  deletion_protection = true
  labels              = local.labels

  scaling {
    max_instance_count = var.api_max_instances
  }

  template {
    service_account                  = google_service_account.api.email
    max_instance_request_concurrency = 8
    timeout                          = "30s"

    scaling {
      min_instance_count = 0
    }

    containers {
      image = var.api_image

      ports {
        container_port = 8080
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "512Mi"
        }
      }

      env {
        name  = "ZENO_API_BIND"
        value = "0.0.0.0:8080"
      }

      env {
        name  = "ZENO_ALLOW_REMOTE_DATABASE"
        value = "true"
      }

      env {
        name  = "DATABASE_MAX_CONNECTIONS"
        value = "4"
      }

      env {
        name  = "DATABASE_ACQUIRE_TIMEOUT_SECONDS"
        value = "15"
      }

      # Span export is opt-in. The endpoint must be the bare host: the SDK appends
      # the signal path itself, and a value that already carried a path would be
      # posted to verbatim.
      env {
        name  = "OTEL_EXPORTER_OTLP_ENDPOINT"
        value = "https://telemetry.googleapis.com"
      }

      # Recorded on every span as gcp.project_id, which is how Cloud Trace files
      # a span under a project.
      env {
        name  = "GOOGLE_CLOUD_PROJECT"
        value = var.project_id
      }

      dynamic "env" {
        for_each = var.neon_auth_base_url == "" ? [] : [var.neon_auth_base_url]
        content {
          name  = "NEON_AUTH_BASE_URL"
          value = env.value
        }
      }

      dynamic "env" {
        for_each = var.neon_auth_jwks_url == "" ? [] : [var.neon_auth_jwks_url]
        content {
          name  = "NEON_AUTH_JWKS_URL"
          value = env.value
        }
      }

      env {
        name = "DATABASE_URL"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.runtime_database_url.secret_id
            version = var.runtime_database_secret_version
          }
        }
      }

      # Order confirmation issues each unit its own credential. The key must come
      # from Secret Manager: with it absent the API still serves, but confirmation
      # fails closed, which is a silent loss of the product's main feature.
      env {
        name = "ZENO_TICKET_CREDENTIAL_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.ticket_credential_key.secret_id
            version = var.ticket_credential_secret_version
          }
        }
      }

      startup_probe {
        initial_delay_seconds = 0
        timeout_seconds       = 2
        period_seconds        = 5
        failure_threshold     = 12

        http_get {
          path = "/health/ready"
          port = 8080
        }
      }

      liveness_probe {
        initial_delay_seconds = 5
        timeout_seconds       = 2
        period_seconds        = 10
        failure_threshold     = 3

        http_get {
          path = "/health/live"
          port = 8080
        }
      }
    }
  }

  depends_on = [
    google_project_service.required,
    google_secret_manager_secret_iam_member.api_runtime_database,
  ]

  lifecycle {
    precondition {
      condition     = var.api_image != ""
      error_message = "Set api_image to an immutable Artifact Registry image before creating Cloud Run workloads."
    }
    precondition {
      condition     = var.runtime_database_secret_version != ""
      error_message = "Set an existing pinned runtime database secret version before creating Cloud Run workloads."
    }

    # Rollouts are owned by the deploy pipeline, which pushes an immutable digest
    # and updates the running revision in place. Without this the next tofu apply
    # would silently roll the live revision back to the digest pinned in tfvars.
    ignore_changes = [template[0].containers[0].image]
  }
}

resource "google_cloud_run_v2_service" "web" {
  count = var.deploy_workloads ? 1 : 0

  project              = var.project_id
  name                 = "zeno-${var.environment}-web"
  location             = var.region
  ingress              = var.publish_web ? "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER" : "INGRESS_TRAFFIC_ALL"
  default_uri_disabled = var.publish_web
  deletion_protection  = true
  labels               = local.labels

  scaling {
    max_instance_count = 3
  }

  template {
    service_account                  = google_service_account.web.email
    max_instance_request_concurrency = 40
    timeout                          = "60s"

    scaling {
      min_instance_count = 0
    }

    containers {
      image = var.web_image

      ports {
        container_port = 3000
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "1Gi"
        }
      }

      env {
        name  = "ZENO_API_ORIGIN"
        value = google_cloud_run_v2_service.api[0].uri
      }

      env {
        name  = "ZENO_API_AUDIENCE"
        value = google_cloud_run_v2_service.api[0].uri
      }

      # Span export is opt-in. The endpoint must be the bare host: the SDK appends
      # the signal path itself, and a value that already carried a path would be
      # posted to verbatim. This is the same pair of variables the API carries, so
      # both tiers resolve the endpoint by one rule.
      env {
        name  = "OTEL_EXPORTER_OTLP_ENDPOINT"
        value = "https://telemetry.googleapis.com"
      }

      env {
        name  = "GOOGLE_CLOUD_PROJECT"
        value = var.project_id
      }

      dynamic "env" {
        for_each = var.neon_auth_base_url == "" ? [] : [var.neon_auth_base_url]
        content {
          name  = "NEON_AUTH_BASE_URL"
          value = env.value
        }
      }

      dynamic "env" {
        for_each = var.web_auth_cookie_secret_version == "" ? [] : [var.web_auth_cookie_secret_version]
        content {
          name = "NEON_AUTH_COOKIE_SECRET"
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.web_auth_cookie_secret.secret_id
              version = env.value
            }
          }
        }
      }

      dynamic "env" {
        for_each = var.web_storage_access_key_secret_version == "" ? [] : [var.web_storage_access_key_secret_version]
        content {
          name = "AWS_ACCESS_KEY_ID"
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.web_storage_access_key.secret_id
              version = env.value
            }
          }
        }
      }

      dynamic "env" {
        for_each = var.web_storage_secret_access_key_secret_version == "" ? [] : [var.web_storage_secret_access_key_secret_version]
        content {
          name = "AWS_SECRET_ACCESS_KEY"
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.web_storage_secret_access_key.secret_id
              version = env.value
            }
          }
        }
      }

      dynamic "env" {
        for_each = var.neon_object_storage_endpoint_url == "" ? [] : [var.neon_object_storage_endpoint_url]
        content {
          name  = "AWS_ENDPOINT_URL_S3"
          value = env.value
        }
      }

      dynamic "env" {
        for_each = var.neon_object_storage_region == "" ? [] : [var.neon_object_storage_region]
        content {
          name  = "AWS_REGION"
          value = env.value
        }
      }

      startup_probe {
        initial_delay_seconds = 0
        timeout_seconds       = 2
        period_seconds        = 5
        failure_threshold     = 12

        tcp_socket {
          port = 3000
        }
      }
    }
  }

  depends_on = [
    google_project_service.required,
    google_cloud_run_v2_service_iam_member.api_invoker,
    google_secret_manager_secret_iam_member.web_auth_cookie_secret,
    google_secret_manager_secret_iam_member.web_storage_access_key,
    google_secret_manager_secret_iam_member.web_storage_secret_access_key,
  ]

  lifecycle {
    precondition {
      condition     = var.web_image != ""
      error_message = "Set web_image to an immutable Artifact Registry image before creating Cloud Run workloads."
    }
    precondition {
      condition     = !var.publish_web || var.expiry_schedule_enabled
      error_message = "Publishing the reservation demo requires the expiry Cloud Scheduler trigger; otherwise public holds will not be released automatically."
    }
    precondition {
      condition     = !var.publish_web || trimspace(var.public_domain) != ""
      error_message = "Public publishing requires a custom HTTPS domain."
    }
    precondition {
      condition     = !var.production_release || (var.publish_web && var.expiry_schedule_enabled && trimspace(var.public_domain) != "" && !var.public_reservation_rate_limit_preview && length(var.monitoring_notification_channels) > 0 && startswith(var.neon_auth_base_url, "https://") && startswith(var.neon_auth_jwks_url, "https://") && var.web_auth_cookie_secret_version != "" && startswith(var.neon_object_storage_endpoint_url, "https://") && var.neon_object_storage_region != "" && var.web_storage_access_key_secret_version != "" && var.web_storage_secret_access_key_secret_version != "")
      error_message = "A production release requires public web, scheduled hold expiry, a custom HTTPS domain, an enforced Cloud Armor reservation rate limit, alert delivery, configured Neon Auth, and pinned Neon Object Storage credentials."
    }

    ignore_changes = [template[0].containers[0].image]
  }
}

resource "google_cloud_run_v2_service_iam_member" "api_invoker" {
  count = var.deploy_workloads ? 1 : 0

  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_service.api[0].name
  role     = "roles/run.invoker"
  member   = "serviceAccount:${google_service_account.web.email}"
}

resource "google_cloud_run_v2_service_iam_member" "web_public_invoker" {
  count = var.deploy_workloads && (var.publish_web || var.allow_public_default_uri) ? 1 : 0

  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_service.web[0].name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

resource "google_cloud_run_v2_job" "expiry_worker" {
  count = var.deploy_workloads ? 1 : 0

  project             = var.project_id
  name                = "zeno-${var.environment}-expire-holds"
  location            = var.region
  deletion_protection = true
  labels              = local.labels

  template {
    task_count  = 1
    parallelism = 1

    template {
      service_account = google_service_account.worker.email
      max_retries     = 1
      timeout         = "60s"

      containers {
        image = var.api_image
        args  = ["worker-once"]

        resources {
          limits = {
            cpu    = "1"
            memory = "512Mi"
          }
        }

        env {
          name = "DATABASE_URL"
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.runtime_database_url.secret_id
              version = var.runtime_database_secret_version
            }
          }
        }

        env {
          name  = "ZENO_ALLOW_REMOTE_DATABASE"
          value = "true"
        }

        env {
          name  = "DATABASE_MAX_CONNECTIONS"
          value = "2"
        }

        env {
          name  = "ZENO_WORKER_INSTANCE_ID"
          value = "reservation-expiry-job"
        }

        env {
          name  = "ZENO_WORKER_BATCH_SIZE"
          value = "100"
        }

        env {
          name  = "ZENO_IDEMPOTENCY_CLEANUP_BATCH_SIZE"
          value = "1000"
        }

        # A job run with no upstream caller is exactly the work that has no trace
        # today, so it exports too.
        env {
          name  = "OTEL_EXPORTER_OTLP_ENDPOINT"
          value = "https://telemetry.googleapis.com"
        }

        env {
          name  = "GOOGLE_CLOUD_PROJECT"
          value = var.project_id
        }
      }
    }
  }

  depends_on = [
    google_project_service.required,
    google_secret_manager_secret_iam_member.worker_runtime_database,
  ]

  lifecycle {
    precondition {
      condition     = var.api_image != ""
      error_message = "Set api_image to an immutable Artifact Registry image before creating Cloud Run workloads."
    }
    precondition {
      condition     = var.runtime_database_secret_version != ""
      error_message = "Set an existing pinned runtime database secret version before creating Cloud Run workloads."
    }

    # The pipeline rolls this job forward with each release so the expiry worker
    # always runs the same code as the API it shares a schema with.
    ignore_changes = [template[0].template[0].containers[0].image]
  }
}

resource "google_cloud_run_v2_job_iam_member" "expiry_scheduler_invoker" {
  count = var.deploy_workloads && var.expiry_schedule_enabled ? 1 : 0

  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_job.expiry_worker[0].name
  role     = "roles/run.invoker"
  member   = "serviceAccount:${google_service_account.scheduler.email}"
}

resource "google_cloud_scheduler_job" "expiry_worker" {
  count = var.deploy_workloads && var.expiry_schedule_enabled ? 1 : 0

  project     = var.project_id
  region      = var.region
  name        = "zeno-${var.environment}-expire-holds"
  description = "Run one bounded Zeno reservation-expiry batch."
  schedule    = var.expiry_schedule
  time_zone   = "Etc/UTC"

  http_target {
    http_method = "POST"
    uri         = "https://run.googleapis.com/v2/projects/${var.project_id}/locations/${var.region}/jobs/${google_cloud_run_v2_job.expiry_worker[0].name}:run"
    body        = base64encode("{}")

    headers = {
      "Content-Type" = "application/json"
    }

    oauth_token {
      service_account_email = google_service_account.scheduler.email
    }
  }

  retry_config {
    retry_count = 1
  }

  depends_on = [google_cloud_run_v2_job_iam_member.expiry_scheduler_invoker]
}

resource "google_cloud_run_v2_job" "migrate" {
  count = var.deploy_workloads ? 1 : 0

  project             = var.project_id
  name                = "zeno-${var.environment}-migrate"
  location            = var.region
  deletion_protection = true
  labels              = local.labels

  template {
    parallelism = 1
    task_count  = 1

    template {
      service_account = google_service_account.migration.email
      max_retries     = 0
      timeout         = "600s"

      containers {
        image = var.api_image
        args  = ["migrate"]

        resources {
          limits = {
            cpu    = "1"
            memory = "512Mi"
          }
        }

        env {
          name = "DATABASE_URL"
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.migration_database_url.secret_id
              version = var.migration_database_secret_version
            }
          }
        }

        env {
          name  = "ZENO_ALLOW_REMOTE_DATABASE"
          value = "true"
        }

        env {
          name  = "DATABASE_MAX_CONNECTIONS"
          value = "2"
        }
      }
    }
  }

  depends_on = [
    google_project_service.required,
    google_secret_manager_secret_iam_member.migration_database,
  ]

  lifecycle {
    precondition {
      condition     = var.api_image != ""
      error_message = "Set api_image to an immutable Artifact Registry image before creating Cloud Run workloads."
    }
    precondition {
      condition     = var.migration_database_secret_version != ""
      error_message = "Set an existing pinned migration database secret version before creating Cloud Run workloads."
    }

    # The pipeline updates this job's image and executes it before rolling the
    # API forward, so migrations for a release always run ahead of the code that
    # depends on them.
    ignore_changes = [template[0].template[0].containers[0].image]
  }
}
