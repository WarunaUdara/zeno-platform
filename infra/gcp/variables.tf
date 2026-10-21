variable "project_id" {
  description = "Existing GCP project to deploy Zeno into."
  type        = string
}

variable "region" {
  description = "GCP region for Cloud Run and Artifact Registry."
  type        = string
  default     = "asia-southeast1"
}

variable "environment" {
  description = "Short environment label used for resource names and labels."
  type        = string
  default     = "demo"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{1,11}$", var.environment))
    error_message = "environment must be 2-12 lowercase letters, digits, or hyphens and start with a letter."
  }
}

variable "deploy_workloads" {
  description = "Create Cloud Run services, scheduled expiry and migration jobs after foundation and secrets are ready."
  type        = bool
  default     = false
}

variable "publish_web" {
  description = "Publish the web service behind the HTTPS load balancer and Cloud Armor after migrations and smoke tests pass."
  type        = bool
  default     = false
}

variable "allow_public_default_uri" {
  description = "Expose the web service on its default Cloud Run run.app URL so the frontend can be opened before a domain exists. This is deliberately not a production posture: with no load balancer there is no Cloud Armor in front of the public reservation endpoint, so the only abuse bound is the service's max_instance_count."
  type        = bool
  default     = false

  validation {
    condition     = !var.allow_public_default_uri || !var.publish_web
    error_message = "publish_web already serves the frontend publicly and disables the default run.app URI, so allow_public_default_uri would be configured but unreachable. Set exactly one of them."
  }
}

variable "production_release" {
  description = "Assert infrastructure wiring for authenticated production organizer access. This does not certify product launch, payment, or provider-backed auth behavior."
  type        = bool
  default     = false

  validation {
    condition = !var.production_release || (
      var.deploy_workloads &&
      var.publish_web &&
      startswith(var.neon_auth_base_url, "https://") &&
      startswith(var.neon_auth_jwks_url, "https://") &&
      var.web_auth_cookie_secret_version != "" &&
      startswith(var.neon_object_storage_endpoint_url, "https://") &&
      var.neon_object_storage_region != "" &&
      var.web_storage_access_key_secret_version != "" &&
      var.web_storage_secret_access_key_secret_version != ""
    )
    error_message = "production_release requires public workloads, Neon Auth configuration, a pinned auth cookie secret, and pinned Neon Object Storage credentials with an HTTPS endpoint and supported region."
  }
}

variable "monitoring_notification_channels" {
  description = "Fully qualified names of pre-created Cloud Monitoring notification channels for service and expiry-job alerts. Set at least one before asserting production_release."
  type        = list(string)
  default     = []

  validation {
    condition = (
      length(distinct(var.monitoring_notification_channels)) == length(var.monitoring_notification_channels) &&
      alltrue([
        for channel in var.monitoring_notification_channels :
        startswith(channel, "projects/${var.project_id}/notificationChannels/") &&
        length(trimprefix(channel, "projects/${var.project_id}/notificationChannels/")) > 0 &&
        !strcontains(trimprefix(channel, "projects/${var.project_id}/notificationChannels/"), "/")
      ])
    )
    error_message = "Monitoring notification channels must be unique resource names in project_id using projects/PROJECT_ID/notificationChannels/CHANNEL_ID format."
  }
}

variable "public_domain" {
  description = "DNS name for the public HTTPS frontend. Required when publish_web is true; point its A record at the public_web_ip output before certificate provisioning completes."
  type        = string
  default     = ""

  validation {
    condition     = var.public_domain == "" || (var.public_domain == trimspace(var.public_domain) && length(var.public_domain) <= 253 && can(regex("^([A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?\\.)+[A-Za-z]{2,63}$", var.public_domain)))
    error_message = "public_domain must be a plain fully qualified DNS hostname without whitespace or a URL scheme."
  }
}

variable "public_reservation_rate_limit_count" {
  description = "Starter Cloud Armor per-IP reservation POST threshold. Tune from preview logs and shared-network testing; this is abuse mitigation, not a strict quota."
  type        = number
  default     = 10

  validation {
    condition     = var.public_reservation_rate_limit_count >= 1 && var.public_reservation_rate_limit_count <= 10000 && floor(var.public_reservation_rate_limit_count) == var.public_reservation_rate_limit_count
    error_message = "public_reservation_rate_limit_count must be an integer from 1 to 10000."
  }
}

variable "public_reservation_rate_limit_interval_sec" {
  description = "Cloud Armor rate-limit interval in seconds."
  type        = number
  default     = 60

  validation {
    condition     = contains([10, 30, 60, 120, 180, 240, 300, 600, 900, 1200, 1800, 2700, 3600], var.public_reservation_rate_limit_interval_sec)
    error_message = "Use a Cloud Armor supported rate-limit interval."
  }
}

variable "public_reservation_rate_limit_preview" {
  description = "Log Cloud Armor rate-limit matches without enforcing them. Public publishing requires this to be false."
  type        = bool
  default     = false
}

variable "api_image" {
  description = "Immutable Artifact Registry image reference for the Rust API and worker. Required when deploy_workloads is true."
  type        = string
  default     = ""
}

variable "web_image" {
  description = "Immutable Artifact Registry image reference for the Next.js web service. Required when deploy_workloads is true."
  type        = string
  default     = ""
}

variable "neon_auth_base_url" {
  description = "HTTPS Neon Auth issuer base URL. Configure from the isolated Neon project; required for production_release."
  type        = string
  default     = ""

  validation {
    condition = (
      (var.neon_auth_base_url == "" && var.neon_auth_jwks_url == "") ||
      (var.neon_auth_base_url != "" && var.neon_auth_jwks_url != "")
      ) && (
      var.neon_auth_base_url == "" || (
        var.neon_auth_base_url == trimspace(var.neon_auth_base_url) &&
        startswith(var.neon_auth_base_url, "https://") &&
        !strcontains(var.neon_auth_base_url, "@") &&
        !strcontains(var.neon_auth_base_url, "?") &&
        !strcontains(var.neon_auth_base_url, "#") &&
        !strcontains(var.neon_auth_base_url, " ")
      )
    )
    error_message = "Neon Auth base and JWKS URLs must be configured together; the base URL must be HTTPS without credentials, query, fragment, or whitespace."
  }
}

variable "neon_auth_jwks_url" {
  description = "HTTPS Neon Auth JWKS endpoint used by the API to verify identity tokens; required for production_release."
  type        = string
  default     = ""

  validation {
    condition = var.neon_auth_jwks_url == "" || (
      var.neon_auth_jwks_url == trimspace(var.neon_auth_jwks_url) &&
      startswith(var.neon_auth_jwks_url, "https://") &&
      !strcontains(var.neon_auth_jwks_url, "@") &&
      !strcontains(var.neon_auth_jwks_url, "?") &&
      !strcontains(var.neon_auth_jwks_url, "#") &&
      !strcontains(var.neon_auth_jwks_url, " ")
    )
    error_message = "neon_auth_jwks_url must be an HTTPS URL without credentials, query, fragment, or whitespace."
  }
}

variable "web_auth_cookie_secret_version" {
  description = "Pinned numeric Secret Manager version of a unique 32+ character Neon Auth cookie secret. Required for production_release."
  type        = string
  default     = ""

  validation {
    condition     = var.web_auth_cookie_secret_version == "" || can(regex("^[1-9][0-9]*$", var.web_auth_cookie_secret_version))
    error_message = "web_auth_cookie_secret_version must be empty or a pinned positive numeric Secret Manager version."
  }
}

variable "neon_object_storage_endpoint_url" {
  description = "HTTPS path-style endpoint for the isolated Neon Object Storage branch; public configuration, not a credential."
  type        = string
  default     = ""

  validation {
    condition = var.neon_object_storage_endpoint_url == "" || (
      var.neon_object_storage_endpoint_url == trimspace(var.neon_object_storage_endpoint_url) &&
      startswith(var.neon_object_storage_endpoint_url, "https://") &&
      !strcontains(var.neon_object_storage_endpoint_url, "@") &&
      !strcontains(var.neon_object_storage_endpoint_url, "?") &&
      !strcontains(var.neon_object_storage_endpoint_url, "#") &&
      !strcontains(var.neon_object_storage_endpoint_url, " ")
    )
    error_message = "neon_object_storage_endpoint_url must be an HTTPS URL without credentials, query, fragment, or whitespace."
  }
}

variable "neon_object_storage_region" {
  description = "AWS region of the Neon Object Storage branch endpoint."
  type        = string
  default     = ""

  validation {
    condition = var.neon_object_storage_region == "" || contains([
      "ap-southeast-1",
      "us-east-1",
      "us-east-2",
      "eu-central-1",
    ], var.neon_object_storage_region)
    error_message = "neon_object_storage_region must be a supported Neon Object Storage region."
  }
}

variable "web_storage_access_key_secret_version" {
  description = "Pinned Secret Manager version for the web-only Neon Object Storage access key."
  type        = string
  default     = ""

  validation {
    condition     = var.web_storage_access_key_secret_version == "" || can(regex("^[1-9][0-9]*$", var.web_storage_access_key_secret_version))
    error_message = "web_storage_access_key_secret_version must be empty or a pinned positive numeric version."
  }
}

variable "web_storage_secret_access_key_secret_version" {
  description = "Pinned Secret Manager version for the web-only Neon Object Storage secret key."
  type        = string
  default     = ""

  validation {
    condition     = var.web_storage_secret_access_key_secret_version == "" || can(regex("^[1-9][0-9]*$", var.web_storage_secret_access_key_secret_version))
    error_message = "web_storage_secret_access_key_secret_version must be empty or a pinned positive numeric version."
  }
}

variable "runtime_database_secret_version" {
  description = "Pinned Secret Manager version for the least-privilege pooled runtime DATABASE_URL."
  type        = string
  default     = "1"
}

variable "ticket_credential_secret_version" {
  description = "Pinned Secret Manager version for ZENO_TICKET_CREDENTIAL_KEY, the AES-256-GCM key that re-displays issued ticket credentials. Without it order confirmation and ticket issuance are disabled."
  type        = string
  default     = "1"
}

variable "migration_database_secret_version" {
  description = "Pinned Secret Manager version for the unpooled, least-privilege migration DATABASE_URL."
  type        = string
  default     = "1"
}

variable "api_max_instances" {
  description = "Hard upper bound on API instances to protect the Neon connection budget."
  type        = number
  default     = 3

  validation {
    condition = (
      var.api_max_instances >= 1 &&
      var.api_max_instances <= 10 &&
      floor(var.api_max_instances) == var.api_max_instances
    )
    error_message = "api_max_instances must be between 1 and 10 until load and connection budgets are measured."
  }
}

variable "expiry_schedule_enabled" {
  description = "Create the Cloud Scheduler trigger for the bounded reservation-expiry Cloud Run Job."
  type        = bool
  default     = true
}

variable "expiry_schedule" {
  description = "Cloud Scheduler cron schedule for reservation expiry. The default runs once per minute."
  type        = string
  default     = "* * * * *"

  validation {
    condition     = trimspace(var.expiry_schedule) != ""
    error_message = "expiry_schedule must be a non-empty unix-cron expression."
  }
}
