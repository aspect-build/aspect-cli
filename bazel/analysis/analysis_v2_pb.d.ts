// @generated by protoc-gen-es v2.2.3 with parameter "keep_empty_files=true,target=js+dts,js_import_style=module,import_extension=.js"
// @generated from file bazel/analysis/analysis_v2.proto (package bazel, syntax proto3)
/* eslint-disable */

import type { GenFile, GenMessage } from "@bufbuild/protobuf/codegenv1";
import type { Message } from "@bufbuild/protobuf";
import type { Target as Target$1 } from "../query/build_pb.js";

/**
 * Describes the file bazel/analysis/analysis_v2.proto.
 */
export declare const file_bazel_analysis_analysis_v2: GenFile;

/**
 * @generated from message bazel.ActionGraphContainer
 */
export declare type ActionGraphContainer = Message<"bazel.ActionGraphContainer"> & {
  /**
   * @generated from field: repeated bazel.Artifact artifacts = 1;
   */
  artifacts: Artifact[];

  /**
   * @generated from field: repeated bazel.Action actions = 2;
   */
  actions: Action[];

  /**
   * @generated from field: repeated bazel.Target targets = 3;
   */
  targets: Target[];

  /**
   * @generated from field: repeated bazel.DepSetOfFiles dep_set_of_files = 4;
   */
  depSetOfFiles: DepSetOfFiles[];

  /**
   * @generated from field: repeated bazel.Configuration configuration = 5;
   */
  configuration: Configuration[];

  /**
   * @generated from field: repeated bazel.AspectDescriptor aspect_descriptors = 6;
   */
  aspectDescriptors: AspectDescriptor[];

  /**
   * @generated from field: repeated bazel.RuleClass rule_classes = 7;
   */
  ruleClasses: RuleClass[];

  /**
   * @generated from field: repeated bazel.PathFragment path_fragments = 8;
   */
  pathFragments: PathFragment[];
};

/**
 * Describes the message bazel.ActionGraphContainer.
 * Use `create(ActionGraphContainerSchema)` to create a new message.
 */
export declare const ActionGraphContainerSchema: GenMessage<ActionGraphContainer>;

/**
 * @generated from message bazel.Artifact
 */
export declare type Artifact = Message<"bazel.Artifact"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: uint32 path_fragment_id = 2;
   */
  pathFragmentId: number;

  /**
   * @generated from field: bool is_tree_artifact = 3;
   */
  isTreeArtifact: boolean;
};

/**
 * Describes the message bazel.Artifact.
 * Use `create(ArtifactSchema)` to create a new message.
 */
export declare const ArtifactSchema: GenMessage<Artifact>;

/**
 * @generated from message bazel.Action
 */
export declare type Action = Message<"bazel.Action"> & {
  /**
   * @generated from field: uint32 target_id = 1;
   */
  targetId: number;

  /**
   * @generated from field: repeated uint32 aspect_descriptor_ids = 2;
   */
  aspectDescriptorIds: number[];

  /**
   * @generated from field: string action_key = 3;
   */
  actionKey: string;

  /**
   * @generated from field: string mnemonic = 4;
   */
  mnemonic: string;

  /**
   * @generated from field: uint32 configuration_id = 5;
   */
  configurationId: number;

  /**
   * @generated from field: repeated string arguments = 6;
   */
  arguments: string[];

  /**
   * @generated from field: repeated bazel.KeyValuePair environment_variables = 7;
   */
  environmentVariables: KeyValuePair[];

  /**
   * @generated from field: repeated uint32 input_dep_set_ids = 8;
   */
  inputDepSetIds: number[];

  /**
   * @generated from field: repeated uint32 scheduling_dep_dep_set_ids = 20;
   */
  schedulingDepDepSetIds: number[];

  /**
   * @generated from field: repeated uint32 output_ids = 9;
   */
  outputIds: number[];

  /**
   * @generated from field: bool discovers_inputs = 10;
   */
  discoversInputs: boolean;

  /**
   * @generated from field: repeated bazel.KeyValuePair execution_info = 11;
   */
  executionInfo: KeyValuePair[];

  /**
   * @generated from field: repeated bazel.ParamFile param_files = 12;
   */
  paramFiles: ParamFile[];

  /**
   * @generated from field: uint32 primary_output_id = 13;
   */
  primaryOutputId: number;

  /**
   * @generated from field: string execution_platform = 14;
   */
  executionPlatform: string;

  /**
   * @generated from field: string template_content = 15;
   */
  templateContent: string;

  /**
   * @generated from field: repeated bazel.KeyValuePair substitutions = 16;
   */
  substitutions: KeyValuePair[];

  /**
   * @generated from field: string file_contents = 17;
   */
  fileContents: string;

  /**
   * @generated from field: string unresolved_symlink_target = 18;
   */
  unresolvedSymlinkTarget: string;

  /**
   * @generated from field: bool is_executable = 19;
   */
  isExecutable: boolean;
};

/**
 * Describes the message bazel.Action.
 * Use `create(ActionSchema)` to create a new message.
 */
export declare const ActionSchema: GenMessage<Action>;

/**
 * @generated from message bazel.Target
 */
export declare type Target = Message<"bazel.Target"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: string label = 2;
   */
  label: string;

  /**
   * @generated from field: uint32 rule_class_id = 3;
   */
  ruleClassId: number;
};

/**
 * Describes the message bazel.Target.
 * Use `create(TargetSchema)` to create a new message.
 */
export declare const TargetSchema: GenMessage<Target>;

/**
 * @generated from message bazel.RuleClass
 */
export declare type RuleClass = Message<"bazel.RuleClass"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: string name = 2;
   */
  name: string;
};

/**
 * Describes the message bazel.RuleClass.
 * Use `create(RuleClassSchema)` to create a new message.
 */
export declare const RuleClassSchema: GenMessage<RuleClass>;

/**
 * @generated from message bazel.AspectDescriptor
 */
export declare type AspectDescriptor = Message<"bazel.AspectDescriptor"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: string name = 2;
   */
  name: string;

  /**
   * @generated from field: repeated bazel.KeyValuePair parameters = 3;
   */
  parameters: KeyValuePair[];
};

/**
 * Describes the message bazel.AspectDescriptor.
 * Use `create(AspectDescriptorSchema)` to create a new message.
 */
export declare const AspectDescriptorSchema: GenMessage<AspectDescriptor>;

/**
 * @generated from message bazel.DepSetOfFiles
 */
export declare type DepSetOfFiles = Message<"bazel.DepSetOfFiles"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: repeated uint32 transitive_dep_set_ids = 2;
   */
  transitiveDepSetIds: number[];

  /**
   * @generated from field: repeated uint32 direct_artifact_ids = 3;
   */
  directArtifactIds: number[];
};

/**
 * Describes the message bazel.DepSetOfFiles.
 * Use `create(DepSetOfFilesSchema)` to create a new message.
 */
export declare const DepSetOfFilesSchema: GenMessage<DepSetOfFiles>;

/**
 * @generated from message bazel.Configuration
 */
export declare type Configuration = Message<"bazel.Configuration"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: string mnemonic = 2;
   */
  mnemonic: string;

  /**
   * @generated from field: string platform_name = 3;
   */
  platformName: string;

  /**
   * @generated from field: string checksum = 4;
   */
  checksum: string;

  /**
   * @generated from field: bool is_tool = 5;
   */
  isTool: boolean;

  /**
   * @generated from field: repeated bazel.Fragment fragments = 6;
   */
  fragments: Fragment[];

  /**
   * @generated from field: repeated bazel.FragmentOptions fragment_options = 7;
   */
  fragmentOptions: FragmentOptions[];
};

/**
 * Describes the message bazel.Configuration.
 * Use `create(ConfigurationSchema)` to create a new message.
 */
export declare const ConfigurationSchema: GenMessage<Configuration>;

/**
 * @generated from message bazel.Fragment
 */
export declare type Fragment = Message<"bazel.Fragment"> & {
  /**
   * @generated from field: string name = 1;
   */
  name: string;

  /**
   * @generated from field: repeated string fragment_option_names = 2;
   */
  fragmentOptionNames: string[];
};

/**
 * Describes the message bazel.Fragment.
 * Use `create(FragmentSchema)` to create a new message.
 */
export declare const FragmentSchema: GenMessage<Fragment>;

/**
 * @generated from message bazel.FragmentOptions
 */
export declare type FragmentOptions = Message<"bazel.FragmentOptions"> & {
  /**
   * @generated from field: string name = 1;
   */
  name: string;

  /**
   * @generated from field: repeated bazel.Option options = 2;
   */
  options: Option[];
};

/**
 * Describes the message bazel.FragmentOptions.
 * Use `create(FragmentOptionsSchema)` to create a new message.
 */
export declare const FragmentOptionsSchema: GenMessage<FragmentOptions>;

/**
 * @generated from message bazel.Option
 */
export declare type Option = Message<"bazel.Option"> & {
  /**
   * @generated from field: optional string name = 1;
   */
  name?: string;

  /**
   * @generated from field: optional string value = 2;
   */
  value?: string;
};

/**
 * Describes the message bazel.Option.
 * Use `create(OptionSchema)` to create a new message.
 */
export declare const OptionSchema: GenMessage<Option>;

/**
 * @generated from message bazel.KeyValuePair
 */
export declare type KeyValuePair = Message<"bazel.KeyValuePair"> & {
  /**
   * @generated from field: string key = 1;
   */
  key: string;

  /**
   * @generated from field: string value = 2;
   */
  value: string;
};

/**
 * Describes the message bazel.KeyValuePair.
 * Use `create(KeyValuePairSchema)` to create a new message.
 */
export declare const KeyValuePairSchema: GenMessage<KeyValuePair>;

/**
 * @generated from message bazel.ConfiguredTarget
 */
export declare type ConfiguredTarget = Message<"bazel.ConfiguredTarget"> & {
  /**
   * @generated from field: blaze_query_aspect_mirror.Target target = 1;
   */
  target?: Target$1;

  /**
   * @generated from field: bazel.Configuration configuration = 2 [deprecated = true];
   * @deprecated
   */
  configuration?: Configuration;

  /**
   * @generated from field: uint32 configuration_id = 3;
   */
  configurationId: number;
};

/**
 * Describes the message bazel.ConfiguredTarget.
 * Use `create(ConfiguredTargetSchema)` to create a new message.
 */
export declare const ConfiguredTargetSchema: GenMessage<ConfiguredTarget>;

/**
 * @generated from message bazel.CqueryResult
 */
export declare type CqueryResult = Message<"bazel.CqueryResult"> & {
  /**
   * @generated from field: repeated bazel.ConfiguredTarget results = 1;
   */
  results: ConfiguredTarget[];

  /**
   * @generated from field: repeated bazel.Configuration configurations = 2;
   */
  configurations: Configuration[];
};

/**
 * Describes the message bazel.CqueryResult.
 * Use `create(CqueryResultSchema)` to create a new message.
 */
export declare const CqueryResultSchema: GenMessage<CqueryResult>;

/**
 * @generated from message bazel.ParamFile
 */
export declare type ParamFile = Message<"bazel.ParamFile"> & {
  /**
   * @generated from field: string exec_path = 1;
   */
  execPath: string;

  /**
   * @generated from field: repeated string arguments = 2;
   */
  arguments: string[];
};

/**
 * Describes the message bazel.ParamFile.
 * Use `create(ParamFileSchema)` to create a new message.
 */
export declare const ParamFileSchema: GenMessage<ParamFile>;

/**
 * @generated from message bazel.PathFragment
 */
export declare type PathFragment = Message<"bazel.PathFragment"> & {
  /**
   * @generated from field: uint32 id = 1;
   */
  id: number;

  /**
   * @generated from field: string label = 2;
   */
  label: string;

  /**
   * @generated from field: uint32 parent_id = 3;
   */
  parentId: number;
};

/**
 * Describes the message bazel.PathFragment.
 * Use `create(PathFragmentSchema)` to create a new message.
 */
export declare const PathFragmentSchema: GenMessage<PathFragment>;

