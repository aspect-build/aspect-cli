// @generated by protoc-gen-es v2.2.3 with parameter "keep_empty_files=true,target=js+dts,js_import_style=module,import_extension=.js"
// @generated from file bazel/options/option_filters.proto (package options, syntax proto3)
/* eslint-disable */

import type { GenEnum, GenFile } from "@bufbuild/protobuf/codegenv1";

/**
 * Describes the file bazel/options/option_filters.proto.
 */
export declare const file_bazel_options_option_filters: GenFile;

/**
 * @generated from enum options.OptionEffectTag
 */
export enum OptionEffectTag {
  /**
   * @generated from enum value: UNKNOWN = 0;
   */
  UNKNOWN = 0,

  /**
   * @generated from enum value: NO_OP = 1;
   */
  NO_OP = 1,

  /**
   * @generated from enum value: LOSES_INCREMENTAL_STATE = 2;
   */
  LOSES_INCREMENTAL_STATE = 2,

  /**
   * @generated from enum value: CHANGES_INPUTS = 3;
   */
  CHANGES_INPUTS = 3,

  /**
   * @generated from enum value: AFFECTS_OUTPUTS = 4;
   */
  AFFECTS_OUTPUTS = 4,

  /**
   * @generated from enum value: BUILD_FILE_SEMANTICS = 5;
   */
  BUILD_FILE_SEMANTICS = 5,

  /**
   * @generated from enum value: BAZEL_INTERNAL_CONFIGURATION = 6;
   */
  BAZEL_INTERNAL_CONFIGURATION = 6,

  /**
   * @generated from enum value: LOADING_AND_ANALYSIS = 7;
   */
  LOADING_AND_ANALYSIS = 7,

  /**
   * @generated from enum value: EXECUTION = 8;
   */
  EXECUTION = 8,

  /**
   * @generated from enum value: HOST_MACHINE_RESOURCE_OPTIMIZATIONS = 9;
   */
  HOST_MACHINE_RESOURCE_OPTIMIZATIONS = 9,

  /**
   * @generated from enum value: EAGERNESS_TO_EXIT = 10;
   */
  EAGERNESS_TO_EXIT = 10,

  /**
   * @generated from enum value: BAZEL_MONITORING = 11;
   */
  BAZEL_MONITORING = 11,

  /**
   * @generated from enum value: TERMINAL_OUTPUT = 12;
   */
  TERMINAL_OUTPUT = 12,

  /**
   * @generated from enum value: ACTION_COMMAND_LINES = 13;
   */
  ACTION_COMMAND_LINES = 13,

  /**
   * @generated from enum value: TEST_RUNNER = 14;
   */
  TEST_RUNNER = 14,
}

/**
 * Describes the enum options.OptionEffectTag.
 */
export declare const OptionEffectTagSchema: GenEnum<OptionEffectTag>;

/**
 * @generated from enum options.OptionMetadataTag
 */
export enum OptionMetadataTag {
  /**
   * @generated from enum value: EXPERIMENTAL = 0;
   */
  EXPERIMENTAL = 0,

  /**
   * @generated from enum value: INCOMPATIBLE_CHANGE = 1;
   */
  INCOMPATIBLE_CHANGE = 1,

  /**
   * @generated from enum value: DEPRECATED = 2;
   */
  DEPRECATED = 2,

  /**
   * @generated from enum value: HIDDEN = 3;
   */
  HIDDEN = 3,

  /**
   * @generated from enum value: INTERNAL = 4;
   */
  INTERNAL = 4,

  /**
   * @generated from enum value: IMMUTABLE = 7;
   */
  IMMUTABLE = 7,
}

/**
 * Describes the enum options.OptionMetadataTag.
 */
export declare const OptionMetadataTagSchema: GenEnum<OptionMetadataTag>;

