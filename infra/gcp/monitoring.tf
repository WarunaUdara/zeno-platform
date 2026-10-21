locals {
  monitored_services = var.deploy_workloads ? {
    web = "zeno-${var.environment}-web"
    api = "zeno-${var.environment}-api"
  } : {}

  # Uptime checks run on Google's public probers, so they must target the address
  # a user resolves. Once the load balancer is published the Cloud Run URI stops
  # accepting public traffic (INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER), and a check
  # pointed at it would fail permanently rather than detecting an outage.
  web_uptime_host = var.deploy_workloads ? (
    var.publish_web
    ? var.public_domain
    : trimprefix(google_cloud_run_v2_service.web[0].uri, "https://")
  ) : ""
}

resource "google_monitoring_alert_policy" "service_error_rate" {
  for_each = local.monitored_services

  project               = var.project_id
  display_name          = "Zeno ${var.environment} ${each.key} 5xx error rate"
  combiner              = "OR"
  notification_channels = var.monitoring_notification_channels
  documentation {
    mime_type = "text/markdown"
    content   = "The Cloud Run service has returned 5xx responses for at least five minutes. Check the service revision logs, API dependency health, and recent deployments. This starter threshold is 5%; tune it after collecting a representative baseline."
  }

  conditions {
    display_name = "5xx responses exceed 5% for five minutes"

    condition_threshold {
      filter = <<-EOT
        resource.type = "cloud_run_revision"
        AND resource.labels.service_name = "${each.value}"
        AND metric.type = "run.googleapis.com/request_count"
        AND metric.labels.response_code_class = "5xx"
      EOT

      denominator_filter = <<-EOT
        resource.type = "cloud_run_revision"
        AND resource.labels.service_name = "${each.value}"
        AND metric.type = "run.googleapis.com/request_count"
      EOT

      comparison              = "COMPARISON_GT"
      threshold_value         = 0.05
      duration                = "300s"
      evaluation_missing_data = "EVALUATION_MISSING_DATA_INACTIVE"

      aggregations {
        alignment_period     = "60s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }

      denominator_aggregations {
        alignment_period     = "60s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }

      trigger {
        count = 1
      }
    }
  }

  user_labels = {
    environment = var.environment
    severity    = "warning"
  }

  depends_on = [google_project_service.required]
}

resource "google_monitoring_alert_policy" "service_latency" {
  for_each = local.monitored_services

  project               = var.project_id
  display_name          = "Zeno ${var.environment} ${each.key} p95 latency"
  combiner              = "OR"
  notification_channels = var.monitoring_notification_channels
  documentation {
    mime_type = "text/markdown"
    content   = "Successful Cloud Run requests have exceeded 2.5 seconds at p95 for five minutes. Check the revision latency breakdown, database pool pressure, and dependency calls. This starter threshold is not a published SLO; tune it after collecting a representative baseline."
  }

  conditions {
    display_name = "Successful request p95 exceeds 2.5 seconds for five minutes"

    condition_threshold {
      filter = <<-EOT
        resource.type = "cloud_run_revision"
        AND resource.labels.service_name = "${each.value}"
        AND metric.type = "run.googleapis.com/request_latencies"
        AND metric.labels.response_code_class = "2xx"
      EOT

      comparison              = "COMPARISON_GT"
      threshold_value         = 2500
      duration                = "300s"
      evaluation_missing_data = "EVALUATION_MISSING_DATA_INACTIVE"

      aggregations {
        alignment_period     = "60s"
        per_series_aligner   = "ALIGN_PERCENTILE_95"
        cross_series_reducer = "REDUCE_MAX"
      }

      trigger {
        count = 1
      }
    }
  }

  user_labels = {
    environment = var.environment
    severity    = "warning"
  }

  depends_on = [google_project_service.required]
}

# Availability observed from outside the system.
#
# Every other policy in this file reads a metric the workloads emit about
# themselves, which means they all go quiet in precisely the case that matters
# most: the service is unreachable and nothing is being emitted for the policy to
# evaluate. This check originates on the public internet, so it is the only signal
# in the module that survives a total failure of the service.
resource "google_monitoring_uptime_check_config" "web" {
  count = var.deploy_workloads ? 1 : 0

  project      = var.project_id
  display_name = "Zeno ${var.environment} public entry"
  timeout      = "10s"
  period       = "300s"

  # Two regions so that a single prober's network problem is not reported as an
  # outage, and both are close enough to asia-southeast1 to keep the measured
  # latency meaningful.
  selected_regions = ["ASIA_PACIFIC", "USA"]

  # The host is part of the monitored resource rather than the HTTP check, which
  # is why it is set here and not alongside the path.
  monitored_resource {
    type = "uptime_url"

    labels = {
      host       = local.web_uptime_host
      project_id = var.project_id
    }
  }

  http_check {
    path         = "/events"
    port         = 443
    use_ssl      = true
    validate_ssl = true

    accepted_response_status_codes {
      status_class = "STATUS_CLASS_2XX"
    }
  }
}

resource "google_monitoring_alert_policy" "web_uptime" {
  count = var.deploy_workloads ? 1 : 0

  project               = var.project_id
  display_name          = "Zeno ${var.environment} public entry unreachable"
  combiner              = "OR"
  notification_channels = var.monitoring_notification_channels

  documentation {
    mime_type = "text/markdown"
    content   = "The public entry point has failed its HTTPS check from every prober region for ten minutes. This is the page-worthy kind of alert: users cannot reach the product at all. Confirm from an unrelated network before doing anything else, then check the Cloud Run revision state, the load balancer backend health, and whether a rollout is in flight. Runbook: docs/operations/OBSERVABILITY_RUNBOOK.md."
  }

  conditions {
    display_name = "Public entry fails from every prober region"

    condition_threshold {
      filter = <<-EOT
        resource.type = "uptime_url"
        AND metric.type = "monitoring.googleapis.com/uptime_check/check_passed"
        AND metric.labels.check_id = "${google_monitoring_uptime_check_config.web[0].uptime_check_id}"
      EOT

      # check_passed reports 1 for a passing probe and 0 for a failing one, so the
      # comparison is a count of failures rather than a value of the metric.
      # Counting FALSE values and requiring more than one means a single prober
      # having a bad day does not wake anyone, while losing both regions does.
      comparison      = "COMPARISON_GT"
      threshold_value = 1
      duration        = "600s"

      aggregations {
        alignment_period     = "1200s"
        per_series_aligner   = "ALIGN_NEXT_OLDER"
        cross_series_reducer = "REDUCE_COUNT_FALSE"
        group_by_fields      = ["resource.label.*"]
      }

      trigger {
        count = 1
      }
    }
  }

  user_labels = {
    environment = var.environment

    # The other policies in this file warn. This one pages: nothing else in the
    # system can be degraded in a way that makes the front door itself unopenable.
    severity = "critical"
  }

  depends_on = [google_project_service.required]
}

resource "google_monitoring_alert_policy" "expiry_job_failure" {
  count = var.deploy_workloads ? 1 : 0

  project               = var.project_id
  display_name          = "Zeno ${var.environment} reservation expiry job failure"
  combiner              = "OR"
  notification_channels = var.monitoring_notification_channels

  documentation {
    mime_type = "text/markdown"
    content   = "A reservation-expiry Cloud Run Job execution failed. Inspect the execution logs and the `reservation-expiry-job` worker heartbeat, then verify that the outbox backlog is recovering. Cloud Monitoring publishes this metric with up to 120 seconds of delay."
  }

  conditions {
    display_name = "A reservation expiry job execution failed"

    condition_threshold {
      filter = <<-EOT
        resource.type = "cloud_run_job"
        AND resource.labels.job_name = "zeno-${var.environment}-expire-holds"
        AND metric.type = "run.googleapis.com/job/completed_execution_count"
        AND metric.labels.result = "failed"
      EOT

      # evaluation_missing_data is deliberately left unset for this policy.
      #
      # The metric is a DELTA: one failed execution yields a non-zero value for a
      # single alignment period and nothing afterwards. EVALUATION_MISSING_DATA_INACTIVE
      # is rejected unless duration is non-zero, and any duration long enough to be
      # accepted makes the condition require consecutive non-zero periods, so a
      # one-off failure would never fire. The API error stating that
      # evaluation_missing_data needs a non-zero duration is really telling us this
      # combination cannot express "alert on any single failure".
      #
      # Leaving it unset keeps the default behaviour, where an alert is not cleared
      # just because the metric stopped reporting.
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period   = "60s"
        per_series_aligner = "ALIGN_DELTA"
      }

      trigger {
        count = 1
      }
    }
  }

  user_labels = {
    environment = var.environment
    severity    = "warning"
  }

  depends_on = [google_project_service.required]
}
