output "artifact_registry_repository" {
  description = "Docker repository used by the release build."
  value       = "${var.region}-docker.pkg.dev/${var.project_id}/${google_artifact_registry_repository.images.repository_id}"
}

output "ticket_credential_key_secret_id" {
  description = "Secret Manager secret for ZENO_TICKET_CREDENTIAL_KEY. Add a 32-byte base64 key outside Terraform."
  value       = google_secret_manager_secret.ticket_credential_key.secret_id
}

output "runtime_database_secret_id" {
  description = "Secret Manager secret for the least-privilege pooled runtime DATABASE_URL. Add secret values outside Terraform."
  value       = google_secret_manager_secret.runtime_database_url.secret_id
}

output "migration_database_secret_id" {
  description = "Secret Manager secret for the unpooled migration DATABASE_URL. Add secret values outside Terraform."
  value       = google_secret_manager_secret.migration_database_url.secret_id
}

output "web_auth_cookie_secret_id" {
  description = "Secret Manager secret for the web session cookie signing secret. Add a unique 32+ character value outside Terraform."
  value       = google_secret_manager_secret.web_auth_cookie_secret.secret_id
}

output "web_storage_access_key_secret_id" {
  description = "Secret Manager secret for the web-only Neon Object Storage access key. Add its value outside Terraform."
  value       = google_secret_manager_secret.web_storage_access_key.secret_id
}

output "web_storage_secret_access_key_secret_id" {
  description = "Secret Manager secret for the web-only Neon Object Storage secret key. Add its value outside Terraform."
  value       = google_secret_manager_secret.web_storage_secret_access_key.secret_id
}

output "web_url" {
  description = "Public HTTPS domain when published, otherwise the Cloud Run URL. The run.app URL only accepts traffic when allow_public_default_uri or publish_web enables public invocation."
  value       = var.deploy_workloads ? (var.publish_web ? "https://${var.public_domain}" : google_cloud_run_v2_service.web[0].uri) : null
}

output "public_web_ip" {
  description = "Static IPv4 frontend address for the public load balancer; create a DNS A record for public_domain pointing here."
  value       = var.deploy_workloads && var.publish_web ? google_compute_global_address.web[0].address : null
}

output "api_url" {
  description = "Cloud Run API URL. Only the web runtime service account receives run.invoker."
  value       = var.deploy_workloads ? google_cloud_run_v2_service.api[0].uri : null
}

output "expiry_job_name" {
  description = "Cloud Run Job that processes one bounded batch of expired reservation holds."
  value       = var.deploy_workloads ? google_cloud_run_v2_job.expiry_worker[0].name : null
}

output "expiry_schedule_name" {
  description = "Cloud Scheduler trigger name; null when the trigger is disabled or workloads are not deployed."
  value       = var.deploy_workloads && var.expiry_schedule_enabled ? google_cloud_scheduler_job.expiry_worker[0].name : null
}
