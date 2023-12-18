provider "aws" {
  alias = "workflows"

  region = "us-west-2"

  default_tags {
    tags = {
      (module.aspect_workflows.cost_allocation_tag) = module.aspect_workflows.cost_allocation_tag_value
    }
  }
}

data "aws_ami" "runner_amd64_ami" {
  # Aspect's AWS account 213396452403 provides public Aspect Workflows images for getting started
  # during the trial period. We recommend that all Workflows users build their own AMIs and keep
  # up-to date with patches. See https://docs.aspect.build/v/workflows/install/packer for more info
  # and/or https://github.com/aspect-build/workflows-images for example packer scripts and BUILD
  # targets for building AMIs for Workflows.
  owners      = ["213396452403"]
  most_recent = true
  filter {
    name   = "name"
    values = ["aspect-workflows-al2-minimal-amd64-*"]
  }
}

data "aws_ami" "runner_arm64_ami" {
  # Aspect's AWS account 213396452403 provides public Aspect Workflows images for getting started
  # during the trial period. We recommend that all Workflows users build their own AMIs and keep
  # up-to date with patches. See https://docs.aspect.build/v/workflows/install/packer for more info
  # and/or https://github.com/aspect-build/workflows-images for example packer scripts and BUILD
  # targets for building AMIs for Workflows.
  owners      = ["213396452403"]
  most_recent = true
  filter {
    name   = "name"
    values = ["aspect-workflows-al2-minimal-arm64-*"]
  }
}

module "aspect_workflows" {
  providers = {
    aws = aws.workflows
  }

  # Aspect Workflows terraform module
  source = "https://s3.us-east-2.amazonaws.com/static.aspect.build/aspect/5.9.0-rc.7/workflows/terraform-aws-aspect-workflows.zip"

  # Non-terraform Aspect Workflows release artifacts are pulled from the region specific
  # aspect-artifacts bucket during apply. Aspect will grant your AWS account access to this bucket
  # during the trial setup. The aspect-artifacts bucket used must in the same region as the
  # deployment.
  aspect_artifacts_bucket = "aspect-artifacts-us-west-2"

  # Name of the deployment
  customer_id = "aspect-build/aspect-cli"

  # VPC properties
  vpc_id             = module.vpc.vpc_id
  vpc_subnets        = module.vpc.private_subnets
  vpc_subnets_public = []

  support = {
    # PagerDuty key for this deployment
    pagerduty_integration_key = "51f08be2e7764903d07252bdc0e65acf"
    # Whether or not to allow SSM access to runners
    enable_ssm_access = true
  }

  # Remote cache properties
  remote_cache = {}

  # Delivery properties
  delivery_enabled = true

  # CI properties
  hosts = ["cci"]

  # Warming set definitions
  warming_sets = {
    default = {}
  }

  # Resource types for use by runner groups. Aspect recommends instance types that have nvme drives
  # for large Bazel workflows. See https://aws.amazon.com/ec2/instance-types/ for list of instance
  # types available on AWS.
  resource_types = {
    "default" = {
      instance_types = ["c5ad.xlarge"]
      image_id       = data.aws_ami.runner_amd64_ami.id
    }
    "small-amd64" = {
      instance_types      = ["t3a.micro"]
      image_id            = data.aws_ami.runner_amd64_ami.id
      root_volume_size_gb = 32
    }
    "small-arm64" = {
      instance_types      = ["t4g.micro"]
      image_id            = data.aws_ami.runner_arm64_ami.id
      root_volume_size_gb = 32
    }
    "nano" = {
      instance_types      = ["t4g.nano"]
      image_id            = data.aws_ami.runner_arm64_ami.id
      root_volume_size_gb = 16
    }
  }

  # CircleCI runner group definitions
  cci_runner_groups = {
    # The default runner group is use for the main build & test workflows.
    default = {
      agent_idle_timeout_min    = 1
      max_runners               = 10
      min_runners               = 0
      resource_type             = "default"
      scaling_polling_frequency = 3 # check for queued jobs every 20s
      warming                   = true
    }
    small-amd64 = {
      agent_idle_timeout_min    = 30
      max_runners               = 10
      min_runners               = 0
      resource_type             = "small-amd64"
      scaling_polling_frequency = 3     # check for queued jobs every 20s
      warming                   = false # don't warm for faster bootstrap; these runners won't be running large builds
    }
    small-arm64 = {
      agent_idle_timeout_min    = 30
      max_runners               = 10
      min_runners               = 0
      resource_type             = "small-arm64"
      scaling_polling_frequency = 3     # check for queued jobs every 20s
      warming                   = false # don't warm for faster bootstrap; these runners won't be running large builds
    }
    nano = {
      agent_idle_timeout_min    = 60 * 12
      max_runners               = 10
      min_runners               = 0
      resource_type             = "nano-arm64"
      scaling_polling_frequency = 3     # check for queued jobs every 20s
      warming                   = false # don't warm for faster bootstrap; these runners won't be running large builds
    }
    # The warming runner group is used for the periodic warming job that creates
    # warming archives for use by other runner groups.
    warming = {
      agent_idle_timeout_min = 1
      max_runners            = 1
      min_runners            = 0
      policies               = { warming_manage : module.aspect_workflows.warming_management_policies["default"].arn }
      resource_type          = "default"
    }
  }
}
