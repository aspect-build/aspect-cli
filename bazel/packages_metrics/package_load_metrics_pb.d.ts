// @generated by protoc-gen-es v2.2.3 with parameter "keep_empty_files=true,target=js+dts,js_import_style=module,import_extension=.js"
// @generated from file bazel/packages_metrics/package_load_metrics.proto (package metrics, syntax proto2)
/* eslint-disable */

import type { GenFile, GenMessage } from "@bufbuild/protobuf/codegenv1";
import type { Message } from "@bufbuild/protobuf";
import type { Duration } from "@bufbuild/protobuf/wkt";

/**
 * Describes the file bazel/packages_metrics/package_load_metrics.proto.
 */
export declare const file_bazel_packages_metrics_package_load_metrics: GenFile;

/**
 * @generated from message metrics.PackageLoadMetrics
 */
export declare type PackageLoadMetrics = Message<"metrics.PackageLoadMetrics"> & {
  /**
   * @generated from field: optional string name = 1;
   */
  name: string;

  /**
   * @generated from field: optional google.protobuf.Duration load_duration = 2;
   */
  loadDuration?: Duration;

  /**
   * @generated from field: optional uint64 num_targets = 3;
   */
  numTargets: bigint;

  /**
   * @generated from field: optional uint64 computation_steps = 4;
   */
  computationSteps: bigint;

  /**
   * @generated from field: optional uint64 num_transitive_loads = 5;
   */
  numTransitiveLoads: bigint;

  /**
   * @generated from field: optional uint64 package_overhead = 6;
   */
  packageOverhead: bigint;
};

/**
 * Describes the message metrics.PackageLoadMetrics.
 * Use `create(PackageLoadMetricsSchema)` to create a new message.
 */
export declare const PackageLoadMetricsSchema: GenMessage<PackageLoadMetrics>;

