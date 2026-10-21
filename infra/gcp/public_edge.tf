resource "google_compute_region_network_endpoint_group" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project               = var.project_id
  name                  = "${local.prefix}-web-neg"
  region                = var.region
  network_endpoint_type = "SERVERLESS"

  cloud_run {
    service = google_cloud_run_v2_service.web[0].name
  }
}

resource "google_compute_security_policy" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project     = var.project_id
  name        = "${local.prefix}-web-edge"
  description = "Per-client abuse guard for public Zeno booking endpoints."

  rule {
    action      = "throttle"
    priority    = 1000
    description = "Throttle reservation and confirmation submissions per client IP."

    match {
      expr {
        # Confirm is unauthenticated and mints tickets, so it needs the same
        # per-IP ceiling as the reservation that precedes it.
        expression = "request.method == 'POST' && (request.path == '/api/reservations' || request.path == '/api/reservations/confirm')"
      }
    }

    rate_limit_options {
      conform_action = "allow"
      exceed_action  = "deny(429)"
      enforce_on_key = "IP"

      rate_limit_threshold {
        count        = var.public_reservation_rate_limit_count
        interval_sec = var.public_reservation_rate_limit_interval_sec
      }
    }

    preview = var.public_reservation_rate_limit_preview
  }

  rule {
    action   = "allow"
    priority = 2147483647

    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = ["*"]
      }
    }
  }
}

resource "google_compute_backend_service" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project               = var.project_id
  name                  = "${local.prefix}-web-backend"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  protocol              = "HTTP"
  security_policy       = google_compute_security_policy.web[0].id

  backend {
    group = google_compute_region_network_endpoint_group.web[0].id
  }
}

resource "google_compute_managed_ssl_certificate" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project = var.project_id
  name    = "${local.prefix}-web-cert"

  managed {
    domains = [var.public_domain]
  }
}

resource "google_compute_ssl_policy" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project         = var.project_id
  name            = "${local.prefix}-web-tls"
  profile         = "MODERN"
  min_tls_version = "TLS_1_2"
}

resource "google_compute_url_map" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project         = var.project_id
  name            = "${local.prefix}-web-url-map"
  default_service = google_compute_backend_service.web[0].id
}

resource "google_compute_target_https_proxy" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project          = var.project_id
  name             = "${local.prefix}-web-https-proxy"
  url_map          = google_compute_url_map.web[0].id
  ssl_certificates = [google_compute_managed_ssl_certificate.web[0].id]
  ssl_policy       = google_compute_ssl_policy.web[0].id
}

resource "google_compute_global_address" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project      = var.project_id
  name         = "${local.prefix}-web-ip"
  address_type = "EXTERNAL"
  ip_version   = "IPV4"
}

resource "google_compute_global_forwarding_rule" "web_https" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project               = var.project_id
  name                  = "${local.prefix}-web-https"
  target                = google_compute_target_https_proxy.web[0].id
  ip_address            = google_compute_global_address.web[0].address
  port_range            = "443"
  ip_protocol           = "TCP"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  network_tier          = "PREMIUM"
}

resource "google_compute_url_map" "web_http_redirect" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project = var.project_id
  name    = "${local.prefix}-web-http-redirect"

  default_url_redirect {
    https_redirect         = true
    redirect_response_code = "PERMANENT_REDIRECT"
    strip_query            = false
  }
}

resource "google_compute_target_http_proxy" "web" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project = var.project_id
  name    = "${local.prefix}-web-http-proxy"
  url_map = google_compute_url_map.web_http_redirect[0].id
}

resource "google_compute_global_forwarding_rule" "web_http" {
  count = var.deploy_workloads && var.publish_web ? 1 : 0

  project               = var.project_id
  name                  = "${local.prefix}-web-http"
  target                = google_compute_target_http_proxy.web[0].id
  ip_address            = google_compute_global_address.web[0].address
  port_range            = "80"
  ip_protocol           = "TCP"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  network_tier          = "PREMIUM"
}
