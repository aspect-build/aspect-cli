locals {
  name                = "aspect-workflows"
  api_key_ttl_seconds = 600
  grafana_version     = "9.4"
}

module "grafana" {
  # https://registry.terraform.io/modules/terraform-aws-modules/managed-service-grafana/aws/2.1.0
  source  = "terraform-aws-modules/managed-service-grafana/aws"
  version = "2.1.0"

  # Workspace
  name                      = local.name
  description               = "Aspect Workflows Grafana dashboards"
  account_access_type       = "CURRENT_ACCOUNT"
  authentication_providers  = ["AWS_SSO"]
  permission_type           = "SERVICE_MANAGED"
  data_sources              = ["PROMETHEUS"]
  notification_destinations = []
  associate_license         = false
  grafana_version           = local.grafana_version

  create_iam_role                = true
  iam_role_name                  = "${local.name}-grafana"
  use_iam_role_name_prefix       = true
  iam_role_description           = "Aspect Workflows Managed Grafana IAM role"
  iam_role_path                  = "/grafana/"
  iam_role_force_detach_policies = true
  iam_role_max_session_duration  = 7200
  iam_role_tags                  = { role = true }
}

resource "time_rotating" "rotate" {
  rotation_minutes = (local.api_key_ttl_seconds / 60)
}

resource "time_static" "rotate" {
  rfc3339 = time_rotating.rotate.rfc3339
}

resource "aws_grafana_workspace_api_key" "admin_key" {
  key_name        = "admin"
  key_role        = "ADMIN"
  seconds_to_live = local.api_key_ttl_seconds + 3600
  workspace_id    = module.grafana.workspace_id

  lifecycle {
    replace_triggered_by = [
      time_static.rotate
    ]
  }
}

resource "aws_grafana_workspace_api_key" "editor_key" {
  key_name        = "editor"
  key_role        = "EDITOR"
  seconds_to_live = local.api_key_ttl_seconds + 3600
  workspace_id    = module.grafana.workspace_id

  lifecycle {
    replace_triggered_by = [
      time_static.rotate
    ]
  }
}

resource "aws_grafana_workspace_api_key" "viewer_key" {
  key_name        = "viewer"
  key_role        = "VIEWER"
  seconds_to_live = local.api_key_ttl_seconds + 3600
  workspace_id    = module.grafana.workspace_id

  lifecycle {
    replace_triggered_by = [
      time_static.rotate
    ]
  }
}