provider "aws" {
  alias = "workflows"

  region = "us-west-2"

  default_tags {
    tags = {
      (module.aspect_workflows.cost_allocation_tag) = module.aspect_workflows.cost_allocation_tag_value
    }
  }
}

data "aws_ami" "runner_ami" {
  # Aspect's AWS account 213396452403 provides public Aspect Workflows images for getting started
  # during the trial period. We recommend that all Workflows users build their own AMIs and keep
  # up-to date with patches. See https://docs.aspect.build/v/workflows/install/packer for more info
  # and/or https://github.com/aspect-build/workflows-images for example packer scripts and BUILD
  # targets for building AMIs for Workflows.
  owners      = ["213396452403"]
  most_recent = true
  filter {
    name   = "name"
    values = ["aspect-workflows-al2-minimal-*"]
  }
}

module "aspect_workflows" {
  providers = {
    aws = aws.workflows
  }

  # Aspect Workflows terraform module
  source = "https://s3.us-east-2.amazonaws.com/static.aspect.build/aspect/5.8.0/workflows/terraform-aws-aspect-workflows.zip"

  # Non-terraform Aspect Workflows release artifacts are pulled from the region specific
  # aspect-artifacts bucket during apply. Aspect will grant your AWS account access to this bucket
  # during the trial setup. The aspect-artifacts bucket used must in the same region as the
  # deployment.
  aspect_artifacts_bucket = "aspect-artifacts-us-west-2"

  # Name of the deployment
  customer_id = "aspect-build/aspect-cli"

  # PagerDuty key for this deployment
  pagerduty_integration_key = "51f08be2e7764903d07252bdc0e65acf"

  # VPC properties
  vpc_id             = module.vpc.vpc_id
  vpc_subnets        = module.vpc.private_subnets
  vpc_subnets_public = []

  # Whether or not to allow SSM access to runners
  enable_ssm_access  = true

  # Opt-in to k8s high availabilty (HA) remote cache; this will be default in future releases
  experiments = {
    k8s_remote        = true
    k8s_observability = true
  }

  # Kubernetes remote cache properties
  experimental_remote = {
    cache_shards           = 3
    cache_size_gb          = 384
    load_balancer_replicas = 2
    replicate_cache        = true
  }

  # Kubernetes cluster properties
  k8s_cluster = {
    cluster_version = "1.27"
    min_size        = 1
    max_size        = 10
    desired_size    = 3
    instance_types  = ["t3.large"],
    # The audit log is very chatty, turn 'er down for a bit.
    cluster_enabled_log_types = ["api", "authenticator"]
  }

  # Monitoring properties
  monitoring_enabled = true

  # Delivery properties
  delivery_enabled = false

  # CI properties
  hosts = ["cci"]

  # Warming set definitions
  warming_sets = {
    default  = {}
  }

  # Resource types for use by runner groups
  resource_types = {
    "default" = {
      # Aspect Workflows requires instance types that have nvme drives. See
      # https://aws.amazon.com/ec2/instance-types/ for full list of instance types available on AWS.
      instance_types = ["c5ad.xlarge"]
      image_id       = data.aws_ami.runner_ami.id
    }
  }

  # CircleCI runner group definitions
  cci_runner_groups = {
    # The default runner group is use for the main build & test workflows.
    default = {
      agent_idle_timeout_min    = 1
      job_max_run_time_min      = 5 * 60
      max_runners               = 10
      min_runners               = 0
      resource_type             = "default"
      scaling_polling_frequency = 3  # check for queued jobs every 20s
      warming                   = true
    }
    # The warming runner group is used for the periodic warming job that creates
    # warming archives for use by other runner groups.
    warming = {
      agent_idle_timeout_min = 1
      job_max_run_time_min   = 5 * 60
      max_runners            = 1
      min_runners            = 0
      policies               = { warming_manage : module.aspect_workflows.warming_management_policies["default"].arn }
      resource_type          = "default"
    }
  }
}
