locals {
  cidr = "10.0.0.0/16"
  azs  = ["us-west-2a", "us-west-2b", "us-west-2c"]

  num_azs                 = length(local.azs)
  num_bits_needed_for_azs = ceil(log(local.num_azs, 2))

  private_cidr = cidrsubnet(local.cidr, 1, 0)
  private_subnets = [
    for i in range(local.num_azs) : cidrsubnet(local.private_cidr, local.num_bits_needed_for_azs, i)
  ]

  public_cidr = cidrsubnet(local.cidr, 1, 1)
  public_subnets = [
    for i in range(local.num_azs) : cidrsubnet(local.public_cidr, local.num_bits_needed_for_azs, i)
  ]
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "4.0.2"

  name = "aw_dev_vpc"
  cidr = local.cidr

  azs             = local.azs
  private_subnets = local.private_subnets
  public_subnets  = local.public_subnets

  enable_nat_gateway      = true
  single_nat_gateway      = true
  enable_vpn_gateway      = false
  map_public_ip_on_launch = true
}

module "vpc_endpoints" {
  source  = "terraform-aws-modules/vpc/aws//modules/vpc-endpoints"
  version = "4.0.2"

  vpc_id = module.vpc.vpc_id
  endpoints = {
    s3 = {
      service         = "s3"
      service_type    = "Gateway"
      tags            = { Name = "s3-vpc-endpoint" }
      route_table_ids = module.vpc.private_route_table_ids
    },
  }
}
