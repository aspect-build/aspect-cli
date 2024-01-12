output "grafana_endpoint" {
  value = "https://${module.grafana.workspace_endpoint}"
}

output "grafana_editor_api_key" {
  value = aws_grafana_workspace_api_key.editor_key.key
}

output "grafana_admin_api_key" {
  value = aws_grafana_workspace_api_key.admin_key.key
}