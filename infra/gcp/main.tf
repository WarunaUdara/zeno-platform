locals {
  prefix = "zeno-${var.environment}"
  labels = {
    application = "zeno"
    environment = var.environment
    managed_by  = "opentofu"
  }
}

resource "google_project_service" "required" {
  for_each = toset([
    "artifactregistry.googleapis.com",
    "cloudscheduler.googleapis.com",
    "compute.googleapis.com",
    "iam.googleapis.com",
    "logging.googleapis.com",
    "monitoring.googleapis.com",
    "run.googleapis.com",
    "secretmanager.googleapis.com",
    "telemetry.googleapis.com",
  ])

  project            = var.project_id
  service            = each.value
  disable_on_destroy = false
}

resource "google_artifact_registry_repository" "images" {
  project       = var.project_id
  location      = var.region
  repository_id = "${local.prefix}-images"
  description   = "Immutable Zeno application images"
  format        = "DOCKER"
  labels        = local.labels

  docker_config {
    immutable_tags = true
  }

  cleanup_policies {
    id     = "keep-recent-release-images"
    action = "KEEP"

    most_recent_versions {
      keep_count = 10
    }
  }

  cleanup_policies {
    id     = "delete-untagged-images"
    action = "DELETE"
    condition {
      tag_state  = "UNTAGGED"
      older_than = "604800s"
    }
  }

  depends_on = [google_project_service.required]

  lifecycle {
    precondition {
      condition     = !var.publish_web || var.deploy_workloads
      error_message = "publish_web requires deploy_workloads=true."
    }

    precondition {
      condition     = !var.allow_public_default_uri || var.deploy_workloads
      error_message = "allow_public_default_uri requires deploy_workloads=true, because it grants unauthenticated access to the web service."
    }
  }
}

resource "google_secret_manager_secret" "runtime_database_url" {
  project   = var.project_id
  secret_id = "${local.prefix}-runtime-database-url"
  labels    = local.labels

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  lifecycle {
    prevent_destroy = true
  }

  depends_on = [google_project_service.required]
}

resource "google_secret_manager_secret" "migration_database_url" {
  project   = var.project_id
  secret_id = "${local.prefix}-migration-database-url"
  labels    = local.labels

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  lifecycle {
    prevent_destroy = true
  }

  depends_on = [google_project_service.required]
}

resource "google_secret_manager_secret" "web_auth_cookie_secret" {
  project   = var.project_id
  secret_id = "${local.prefix}-web-auth-cookie-secret"
  labels    = local.labels

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  lifecycle {
    prevent_destroy = true
  }

  depends_on = [google_project_service.required]
}

resource "google_secret_manager_secret" "web_storage_access_key" {
  project   = var.project_id
  secret_id = "${local.prefix}-web-storage-access-key"
  labels    = local.labels

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  lifecycle {
    prevent_destroy = true
  }

  depends_on = [google_project_service.required]
}

resource "google_secret_manager_secret" "web_storage_secret_access_key" {
  project   = var.project_id
  secret_id = "${local.prefix}-web-storage-secret-access-key"
  labels    = local.labels

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  lifecycle {
    prevent_destroy = true
  }

  depends_on = [google_project_service.required]
}

resource "google_secret_manager_secret" "ticket_credential_key" {
  project   = var.project_id
  secret_id = "${local.prefix}-ticket-credential-key"
  labels    = local.labels

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  lifecycle {
    prevent_destroy = true
  }

  depends_on = [google_project_service.required]
}

resource "google_service_account" "web" {
  project      = var.project_id
  account_id   = "${local.prefix}-web"
  display_name = "Zeno web runtime (${var.environment})"
}

resource "google_service_account" "api" {
  project      = var.project_id
  account_id   = "${local.prefix}-api"
  display_name = "Zeno API runtime (${var.environment})"
}

resource "google_service_account" "worker" {
  project      = var.project_id
  account_id   = "${local.prefix}-worker"
  display_name = "Zeno reservation worker (${var.environment})"
}

resource "google_service_account" "scheduler" {
  project      = var.project_id
  account_id   = "${local.prefix}-scheduler"
  display_name = "Zeno expiry job scheduler (${var.environment})"
}

resource "google_service_account" "migration" {
  project      = var.project_id
  account_id   = "${local.prefix}-migration"
  display_name = "Zeno database migration job (${var.environment})"
}

# Span export goes through the Telemetry API, which is a different API from Cloud
# Trace and needs its own role. Granted only to the three identities that run an
# exporting process: the API, the expiry worker, and the web tier. The migration job
# runs a command that produces no spans.
resource "google_project_iam_member" "trace_writer" {
  for_each = {
    api    = google_service_account.api.email
    worker = google_service_account.worker.email
    web    = google_service_account.web.email
  }

  project = var.project_id
  role    = "roles/telemetry.tracesWriter"
  member  = "serviceAccount:${each.value}"
}

resource "google_secret_manager_secret_iam_member" "api_runtime_database" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.runtime_database_url.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.api.email}"
}

# The API is the only identity that may read the ticket credential key. The
# worker never issues tickets, and the web tier never sees it at all.
resource "google_secret_manager_secret_iam_member" "api_ticket_credential_key" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.ticket_credential_key.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.api.email}"
}

resource "google_secret_manager_secret_iam_member" "worker_runtime_database" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.runtime_database_url.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.worker.email}"
}

resource "google_secret_manager_secret_iam_member" "migration_database" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.migration_database_url.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.migration.email}"
}

resource "google_secret_manager_secret_iam_member" "web_auth_cookie_secret" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.web_auth_cookie_secret.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.web.email}"
}

resource "google_secret_manager_secret_iam_member" "web_storage_access_key" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.web_storage_access_key.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.web.email}"
}

resource "google_secret_manager_secret_iam_member" "web_storage_secret_access_key" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.web_storage_secret_access_key.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.web.email}"
}
