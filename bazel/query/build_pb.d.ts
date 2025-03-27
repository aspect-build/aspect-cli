// @generated by protoc-gen-es v2.2.3 with parameter "keep_empty_files=true,target=js+dts,js_import_style=module,import_extension=.js"
// @generated from file bazel/query/build.proto (package blaze_query_aspect_mirror, syntax proto2)
/* eslint-disable */

import type { GenEnum, GenFile, GenMessage } from "@bufbuild/protobuf/codegenv1";
import type { Message } from "@bufbuild/protobuf";

/**
 * Describes the file bazel/query/build.proto.
 */
export declare const file_bazel_query_build: GenFile;

/**
 * @generated from message blaze_query_aspect_mirror.License
 */
export declare type License = Message<"blaze_query_aspect_mirror.License"> & {
  /**
   * @generated from field: repeated string license_type = 1;
   */
  licenseType: string[];

  /**
   * @generated from field: repeated string exception = 2;
   */
  exception: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.License.
 * Use `create(LicenseSchema)` to create a new message.
 */
export declare const LicenseSchema: GenMessage<License>;

/**
 * @generated from message blaze_query_aspect_mirror.StringDictEntry
 */
export declare type StringDictEntry = Message<"blaze_query_aspect_mirror.StringDictEntry"> & {
  /**
   * @generated from field: required string key = 1;
   */
  key: string;

  /**
   * @generated from field: required string value = 2;
   */
  value: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.StringDictEntry.
 * Use `create(StringDictEntrySchema)` to create a new message.
 */
export declare const StringDictEntrySchema: GenMessage<StringDictEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.LabelDictUnaryEntry
 */
export declare type LabelDictUnaryEntry = Message<"blaze_query_aspect_mirror.LabelDictUnaryEntry"> & {
  /**
   * @generated from field: required string key = 1;
   */
  key: string;

  /**
   * @generated from field: required string value = 2;
   */
  value: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.LabelDictUnaryEntry.
 * Use `create(LabelDictUnaryEntrySchema)` to create a new message.
 */
export declare const LabelDictUnaryEntrySchema: GenMessage<LabelDictUnaryEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.LabelListDictEntry
 */
export declare type LabelListDictEntry = Message<"blaze_query_aspect_mirror.LabelListDictEntry"> & {
  /**
   * @generated from field: required string key = 1;
   */
  key: string;

  /**
   * @generated from field: repeated string value = 2;
   */
  value: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.LabelListDictEntry.
 * Use `create(LabelListDictEntrySchema)` to create a new message.
 */
export declare const LabelListDictEntrySchema: GenMessage<LabelListDictEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.LabelKeyedStringDictEntry
 */
export declare type LabelKeyedStringDictEntry = Message<"blaze_query_aspect_mirror.LabelKeyedStringDictEntry"> & {
  /**
   * @generated from field: required string key = 1;
   */
  key: string;

  /**
   * @generated from field: required string value = 2;
   */
  value: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.LabelKeyedStringDictEntry.
 * Use `create(LabelKeyedStringDictEntrySchema)` to create a new message.
 */
export declare const LabelKeyedStringDictEntrySchema: GenMessage<LabelKeyedStringDictEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.StringListDictEntry
 */
export declare type StringListDictEntry = Message<"blaze_query_aspect_mirror.StringListDictEntry"> & {
  /**
   * @generated from field: required string key = 1;
   */
  key: string;

  /**
   * @generated from field: repeated string value = 2;
   */
  value: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.StringListDictEntry.
 * Use `create(StringListDictEntrySchema)` to create a new message.
 */
export declare const StringListDictEntrySchema: GenMessage<StringListDictEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.FilesetEntry
 */
export declare type FilesetEntry = Message<"blaze_query_aspect_mirror.FilesetEntry"> & {
  /**
   * @generated from field: required string source = 1;
   */
  source: string;

  /**
   * @generated from field: required string destination_directory = 2;
   */
  destinationDirectory: string;

  /**
   * @generated from field: optional bool files_present = 7;
   */
  filesPresent: boolean;

  /**
   * @generated from field: repeated string file = 3;
   */
  file: string[];

  /**
   * @generated from field: repeated string exclude = 4;
   */
  exclude: string[];

  /**
   * @generated from field: optional blaze_query_aspect_mirror.FilesetEntry.SymlinkBehavior symlink_behavior = 5 [default = COPY];
   */
  symlinkBehavior: FilesetEntry_SymlinkBehavior;

  /**
   * @generated from field: optional string strip_prefix = 6;
   */
  stripPrefix: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.FilesetEntry.
 * Use `create(FilesetEntrySchema)` to create a new message.
 */
export declare const FilesetEntrySchema: GenMessage<FilesetEntry>;

/**
 * @generated from enum blaze_query_aspect_mirror.FilesetEntry.SymlinkBehavior
 */
export enum FilesetEntry_SymlinkBehavior {
  /**
   * @generated from enum value: COPY = 1;
   */
  COPY = 1,

  /**
   * @generated from enum value: DEREFERENCE = 2;
   */
  DEREFERENCE = 2,
}

/**
 * Describes the enum blaze_query_aspect_mirror.FilesetEntry.SymlinkBehavior.
 */
export declare const FilesetEntry_SymlinkBehaviorSchema: GenEnum<FilesetEntry_SymlinkBehavior>;

/**
 * @generated from message blaze_query_aspect_mirror.Attribute
 */
export declare type Attribute = Message<"blaze_query_aspect_mirror.Attribute"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: optional bool explicitly_specified = 13;
   */
  explicitlySpecified: boolean;

  /**
   * @generated from field: optional bool nodep = 20;
   */
  nodep: boolean;

  /**
   * @generated from field: optional string source_aspect_name = 23;
   */
  sourceAspectName: string;

  /**
   * @generated from field: required blaze_query_aspect_mirror.Attribute.Discriminator type = 2;
   */
  type: Attribute_Discriminator;

  /**
   * @generated from field: optional int32 int_value = 3;
   */
  intValue: number;

  /**
   * @generated from field: optional string string_value = 5;
   */
  stringValue: string;

  /**
   * @generated from field: optional bool boolean_value = 14;
   */
  booleanValue: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.Tristate tristate_value = 15;
   */
  tristateValue: Attribute_Tristate;

  /**
   * @generated from field: repeated string string_list_value = 6;
   */
  stringListValue: string[];

  /**
   * @generated from field: optional blaze_query_aspect_mirror.License license = 7;
   */
  license?: License;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.StringDictEntry string_dict_value = 8;
   */
  stringDictValue: StringDictEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.FilesetEntry fileset_list_value = 9;
   */
  filesetListValue: FilesetEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.LabelListDictEntry label_list_dict_value = 10;
   */
  labelListDictValue: LabelListDictEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.StringListDictEntry string_list_dict_value = 11;
   */
  stringListDictValue: StringListDictEntry[];

  /**
   * @generated from field: repeated int32 int_list_value = 17;
   */
  intListValue: number[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.LabelDictUnaryEntry label_dict_unary_value = 19;
   */
  labelDictUnaryValue: LabelDictUnaryEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.LabelKeyedStringDictEntry label_keyed_string_dict_value = 22;
   */
  labelKeyedStringDictValue: LabelKeyedStringDictEntry[];

  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.SelectorList selector_list = 21;
   */
  selectorList?: Attribute_SelectorList;

  /**
   * @generated from field: repeated bytes DEPRECATED_string_dict_unary_value = 18;
   */
  DEPRECATEDStringDictUnaryValue: Uint8Array[];
};

/**
 * Describes the message blaze_query_aspect_mirror.Attribute.
 * Use `create(AttributeSchema)` to create a new message.
 */
export declare const AttributeSchema: GenMessage<Attribute>;

/**
 * @generated from message blaze_query_aspect_mirror.Attribute.SelectorEntry
 */
export declare type Attribute_SelectorEntry = Message<"blaze_query_aspect_mirror.Attribute.SelectorEntry"> & {
  /**
   * @generated from field: optional string label = 1;
   */
  label: string;

  /**
   * @generated from field: optional bool is_default_value = 16;
   */
  isDefaultValue: boolean;

  /**
   * @generated from field: optional int32 int_value = 2;
   */
  intValue: number;

  /**
   * @generated from field: optional string string_value = 3;
   */
  stringValue: string;

  /**
   * @generated from field: optional bool boolean_value = 4;
   */
  booleanValue: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.Tristate tristate_value = 5;
   */
  tristateValue: Attribute_Tristate;

  /**
   * @generated from field: repeated string string_list_value = 6;
   */
  stringListValue: string[];

  /**
   * @generated from field: optional blaze_query_aspect_mirror.License license = 7;
   */
  license?: License;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.StringDictEntry string_dict_value = 8;
   */
  stringDictValue: StringDictEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.FilesetEntry fileset_list_value = 9;
   */
  filesetListValue: FilesetEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.LabelListDictEntry label_list_dict_value = 10;
   */
  labelListDictValue: LabelListDictEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.StringListDictEntry string_list_dict_value = 11;
   */
  stringListDictValue: StringListDictEntry[];

  /**
   * @generated from field: repeated int32 int_list_value = 13;
   */
  intListValue: number[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.LabelDictUnaryEntry label_dict_unary_value = 15;
   */
  labelDictUnaryValue: LabelDictUnaryEntry[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.LabelKeyedStringDictEntry label_keyed_string_dict_value = 17;
   */
  labelKeyedStringDictValue: LabelKeyedStringDictEntry[];

  /**
   * @generated from field: repeated bytes DEPRECATED_string_dict_unary_value = 14;
   */
  DEPRECATEDStringDictUnaryValue: Uint8Array[];
};

/**
 * Describes the message blaze_query_aspect_mirror.Attribute.SelectorEntry.
 * Use `create(Attribute_SelectorEntrySchema)` to create a new message.
 */
export declare const Attribute_SelectorEntrySchema: GenMessage<Attribute_SelectorEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.Attribute.Selector
 */
export declare type Attribute_Selector = Message<"blaze_query_aspect_mirror.Attribute.Selector"> & {
  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Attribute.SelectorEntry entries = 1;
   */
  entries: Attribute_SelectorEntry[];

  /**
   * @generated from field: optional bool has_default_value = 2;
   */
  hasDefaultValue: boolean;

  /**
   * @generated from field: optional string no_match_error = 3;
   */
  noMatchError: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.Attribute.Selector.
 * Use `create(Attribute_SelectorSchema)` to create a new message.
 */
export declare const Attribute_SelectorSchema: GenMessage<Attribute_Selector>;

/**
 * @generated from message blaze_query_aspect_mirror.Attribute.SelectorList
 */
export declare type Attribute_SelectorList = Message<"blaze_query_aspect_mirror.Attribute.SelectorList"> & {
  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.Discriminator type = 1;
   */
  type: Attribute_Discriminator;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Attribute.Selector elements = 2;
   */
  elements: Attribute_Selector[];
};

/**
 * Describes the message blaze_query_aspect_mirror.Attribute.SelectorList.
 * Use `create(Attribute_SelectorListSchema)` to create a new message.
 */
export declare const Attribute_SelectorListSchema: GenMessage<Attribute_SelectorList>;

/**
 * @generated from enum blaze_query_aspect_mirror.Attribute.Discriminator
 */
export enum Attribute_Discriminator {
  /**
   * @generated from enum value: INTEGER = 1;
   */
  INTEGER = 1,

  /**
   * @generated from enum value: STRING = 2;
   */
  STRING = 2,

  /**
   * @generated from enum value: LABEL = 3;
   */
  LABEL = 3,

  /**
   * @generated from enum value: OUTPUT = 4;
   */
  OUTPUT = 4,

  /**
   * @generated from enum value: STRING_LIST = 5;
   */
  STRING_LIST = 5,

  /**
   * @generated from enum value: LABEL_LIST = 6;
   */
  LABEL_LIST = 6,

  /**
   * @generated from enum value: OUTPUT_LIST = 7;
   */
  OUTPUT_LIST = 7,

  /**
   * @generated from enum value: DISTRIBUTION_SET = 8;
   */
  DISTRIBUTION_SET = 8,

  /**
   * @generated from enum value: LICENSE = 9;
   */
  LICENSE = 9,

  /**
   * @generated from enum value: STRING_DICT = 10;
   */
  STRING_DICT = 10,

  /**
   * @generated from enum value: FILESET_ENTRY_LIST = 11;
   */
  FILESET_ENTRY_LIST = 11,

  /**
   * @generated from enum value: LABEL_LIST_DICT = 12;
   */
  LABEL_LIST_DICT = 12,

  /**
   * @generated from enum value: STRING_LIST_DICT = 13;
   */
  STRING_LIST_DICT = 13,

  /**
   * @generated from enum value: BOOLEAN = 14;
   */
  BOOLEAN = 14,

  /**
   * @generated from enum value: TRISTATE = 15;
   */
  TRISTATE = 15,

  /**
   * @generated from enum value: INTEGER_LIST = 16;
   */
  INTEGER_LIST = 16,

  /**
   * @generated from enum value: UNKNOWN = 18;
   */
  UNKNOWN = 18,

  /**
   * @generated from enum value: LABEL_DICT_UNARY = 19;
   */
  LABEL_DICT_UNARY = 19,

  /**
   * @generated from enum value: SELECTOR_LIST = 20;
   */
  SELECTOR_LIST = 20,

  /**
   * @generated from enum value: LABEL_KEYED_STRING_DICT = 21;
   */
  LABEL_KEYED_STRING_DICT = 21,

  /**
   * @generated from enum value: DEPRECATED_STRING_DICT_UNARY = 17;
   */
  DEPRECATED_STRING_DICT_UNARY = 17,
}

/**
 * Describes the enum blaze_query_aspect_mirror.Attribute.Discriminator.
 */
export declare const Attribute_DiscriminatorSchema: GenEnum<Attribute_Discriminator>;

/**
 * @generated from enum blaze_query_aspect_mirror.Attribute.Tristate
 */
export enum Attribute_Tristate {
  /**
   * @generated from enum value: NO = 0;
   */
  NO = 0,

  /**
   * @generated from enum value: YES = 1;
   */
  YES = 1,

  /**
   * @generated from enum value: AUTO = 2;
   */
  AUTO = 2,
}

/**
 * Describes the enum blaze_query_aspect_mirror.Attribute.Tristate.
 */
export declare const Attribute_TristateSchema: GenEnum<Attribute_Tristate>;

/**
 * @generated from message blaze_query_aspect_mirror.Rule
 */
export declare type Rule = Message<"blaze_query_aspect_mirror.Rule"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: required string rule_class = 2;
   */
  ruleClass: string;

  /**
   * @generated from field: optional string location = 3;
   */
  location: string;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Attribute attribute = 4;
   */
  attribute: Attribute[];

  /**
   * @generated from field: repeated string rule_input = 5;
   */
  ruleInput: string[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.ConfiguredRuleInput configured_rule_input = 15;
   */
  configuredRuleInput: ConfiguredRuleInput[];

  /**
   * @generated from field: repeated string rule_output = 6;
   */
  ruleOutput: string[];

  /**
   * @generated from field: repeated string default_setting = 7;
   */
  defaultSetting: string[];

  /**
   * @generated from field: optional bool DEPRECATED_public_by_default = 9;
   */
  DEPRECATEDPublicByDefault: boolean;

  /**
   * @generated from field: optional bool DEPRECATED_is_skylark = 10;
   */
  DEPRECATEDIsSkylark: boolean;

  /**
   * @generated from field: optional string skylark_environment_hash_code = 12;
   */
  skylarkEnvironmentHashCode: string;

  /**
   * @generated from field: repeated string instantiation_stack = 13;
   */
  instantiationStack: string[];

  /**
   * @generated from field: repeated string definition_stack = 14;
   */
  definitionStack: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.Rule.
 * Use `create(RuleSchema)` to create a new message.
 */
export declare const RuleSchema: GenMessage<Rule>;

/**
 * @generated from message blaze_query_aspect_mirror.ConfiguredRuleInput
 */
export declare type ConfiguredRuleInput = Message<"blaze_query_aspect_mirror.ConfiguredRuleInput"> & {
  /**
   * @generated from field: optional string label = 1;
   */
  label: string;

  /**
   * @generated from field: optional string configuration_checksum = 2;
   */
  configurationChecksum: string;

  /**
   * @generated from field: optional uint32 configuration_id = 3;
   */
  configurationId: number;
};

/**
 * Describes the message blaze_query_aspect_mirror.ConfiguredRuleInput.
 * Use `create(ConfiguredRuleInputSchema)` to create a new message.
 */
export declare const ConfiguredRuleInputSchema: GenMessage<ConfiguredRuleInput>;

/**
 * @generated from message blaze_query_aspect_mirror.RuleSummary
 */
export declare type RuleSummary = Message<"blaze_query_aspect_mirror.RuleSummary"> & {
  /**
   * @generated from field: required blaze_query_aspect_mirror.Rule rule = 1;
   */
  rule?: Rule;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Rule dependency = 2;
   */
  dependency: Rule[];

  /**
   * @generated from field: optional string location = 3;
   */
  location: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.RuleSummary.
 * Use `create(RuleSummarySchema)` to create a new message.
 */
export declare const RuleSummarySchema: GenMessage<RuleSummary>;

/**
 * @generated from message blaze_query_aspect_mirror.PackageGroup
 */
export declare type PackageGroup = Message<"blaze_query_aspect_mirror.PackageGroup"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: repeated string contained_package = 2;
   */
  containedPackage: string[];

  /**
   * @generated from field: repeated string included_package_group = 3;
   */
  includedPackageGroup: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.PackageGroup.
 * Use `create(PackageGroupSchema)` to create a new message.
 */
export declare const PackageGroupSchema: GenMessage<PackageGroup>;

/**
 * @generated from message blaze_query_aspect_mirror.EnvironmentGroup
 */
export declare type EnvironmentGroup = Message<"blaze_query_aspect_mirror.EnvironmentGroup"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: repeated string environment = 2;
   */
  environment: string[];

  /**
   * @generated from field: repeated string default = 3;
   */
  default: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.EnvironmentGroup.
 * Use `create(EnvironmentGroupSchema)` to create a new message.
 */
export declare const EnvironmentGroupSchema: GenMessage<EnvironmentGroup>;

/**
 * @generated from message blaze_query_aspect_mirror.SourceFile
 */
export declare type SourceFile = Message<"blaze_query_aspect_mirror.SourceFile"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: optional string location = 2;
   */
  location: string;

  /**
   * @generated from field: repeated string subinclude = 3;
   */
  subinclude: string[];

  /**
   * @generated from field: repeated string package_group = 4;
   */
  packageGroup: string[];

  /**
   * @generated from field: repeated string visibility_label = 5;
   */
  visibilityLabel: string[];

  /**
   * @generated from field: repeated string feature = 6;
   */
  feature: string[];

  /**
   * @generated from field: optional blaze_query_aspect_mirror.License license = 8;
   */
  license?: License;

  /**
   * @generated from field: optional bool package_contains_errors = 9;
   */
  packageContainsErrors: boolean;
};

/**
 * Describes the message blaze_query_aspect_mirror.SourceFile.
 * Use `create(SourceFileSchema)` to create a new message.
 */
export declare const SourceFileSchema: GenMessage<SourceFile>;

/**
 * @generated from message blaze_query_aspect_mirror.GeneratedFile
 */
export declare type GeneratedFile = Message<"blaze_query_aspect_mirror.GeneratedFile"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: required string generating_rule = 2;
   */
  generatingRule: string;

  /**
   * @generated from field: optional string location = 3;
   */
  location: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.GeneratedFile.
 * Use `create(GeneratedFileSchema)` to create a new message.
 */
export declare const GeneratedFileSchema: GenMessage<GeneratedFile>;

/**
 * @generated from message blaze_query_aspect_mirror.Target
 */
export declare type Target = Message<"blaze_query_aspect_mirror.Target"> & {
  /**
   * @generated from field: required blaze_query_aspect_mirror.Target.Discriminator type = 1;
   */
  type: Target_Discriminator;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.Rule rule = 2;
   */
  rule?: Rule;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.SourceFile source_file = 3;
   */
  sourceFile?: SourceFile;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.GeneratedFile generated_file = 4;
   */
  generatedFile?: GeneratedFile;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.PackageGroup package_group = 5;
   */
  packageGroup?: PackageGroup;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.EnvironmentGroup environment_group = 6;
   */
  environmentGroup?: EnvironmentGroup;
};

/**
 * Describes the message blaze_query_aspect_mirror.Target.
 * Use `create(TargetSchema)` to create a new message.
 */
export declare const TargetSchema: GenMessage<Target>;

/**
 * @generated from enum blaze_query_aspect_mirror.Target.Discriminator
 */
export enum Target_Discriminator {
  /**
   * @generated from enum value: RULE = 1;
   */
  RULE = 1,

  /**
   * @generated from enum value: SOURCE_FILE = 2;
   */
  SOURCE_FILE = 2,

  /**
   * @generated from enum value: GENERATED_FILE = 3;
   */
  GENERATED_FILE = 3,

  /**
   * @generated from enum value: PACKAGE_GROUP = 4;
   */
  PACKAGE_GROUP = 4,

  /**
   * @generated from enum value: ENVIRONMENT_GROUP = 5;
   */
  ENVIRONMENT_GROUP = 5,
}

/**
 * Describes the enum blaze_query_aspect_mirror.Target.Discriminator.
 */
export declare const Target_DiscriminatorSchema: GenEnum<Target_Discriminator>;

/**
 * @generated from message blaze_query_aspect_mirror.QueryResult
 */
export declare type QueryResult = Message<"blaze_query_aspect_mirror.QueryResult"> & {
  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Target target = 1;
   */
  target: Target[];
};

/**
 * Describes the message blaze_query_aspect_mirror.QueryResult.
 * Use `create(QueryResultSchema)` to create a new message.
 */
export declare const QueryResultSchema: GenMessage<QueryResult>;

/**
 * @generated from message blaze_query_aspect_mirror.AllowedRuleClassInfo
 */
export declare type AllowedRuleClassInfo = Message<"blaze_query_aspect_mirror.AllowedRuleClassInfo"> & {
  /**
   * @generated from field: required blaze_query_aspect_mirror.AllowedRuleClassInfo.AllowedRuleClasses policy = 1;
   */
  policy: AllowedRuleClassInfo_AllowedRuleClasses;

  /**
   * @generated from field: repeated string allowed_rule_class = 2;
   */
  allowedRuleClass: string[];
};

/**
 * Describes the message blaze_query_aspect_mirror.AllowedRuleClassInfo.
 * Use `create(AllowedRuleClassInfoSchema)` to create a new message.
 */
export declare const AllowedRuleClassInfoSchema: GenMessage<AllowedRuleClassInfo>;

/**
 * @generated from enum blaze_query_aspect_mirror.AllowedRuleClassInfo.AllowedRuleClasses
 */
export enum AllowedRuleClassInfo_AllowedRuleClasses {
  /**
   * @generated from enum value: ANY = 1;
   */
  ANY = 1,

  /**
   * @generated from enum value: SPECIFIED = 2;
   */
  SPECIFIED = 2,
}

/**
 * Describes the enum blaze_query_aspect_mirror.AllowedRuleClassInfo.AllowedRuleClasses.
 */
export declare const AllowedRuleClassInfo_AllowedRuleClassesSchema: GenEnum<AllowedRuleClassInfo_AllowedRuleClasses>;

/**
 * @generated from message blaze_query_aspect_mirror.AttributeDefinition
 */
export declare type AttributeDefinition = Message<"blaze_query_aspect_mirror.AttributeDefinition"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: required blaze_query_aspect_mirror.Attribute.Discriminator type = 2;
   */
  type: Attribute_Discriminator;

  /**
   * @generated from field: optional bool mandatory = 3;
   */
  mandatory: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.AllowedRuleClassInfo allowed_rule_classes = 4;
   */
  allowedRuleClasses?: AllowedRuleClassInfo;

  /**
   * @generated from field: optional string documentation = 5;
   */
  documentation: string;

  /**
   * @generated from field: optional bool allow_empty = 6;
   */
  allowEmpty: boolean;

  /**
   * @generated from field: optional bool allow_single_file = 7;
   */
  allowSingleFile: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.AttributeValue default = 9;
   */
  default?: AttributeValue;

  /**
   * @generated from field: optional bool executable = 10;
   */
  executable: boolean;

  /**
   * @generated from field: optional bool configurable = 11;
   */
  configurable: boolean;

  /**
   * @generated from field: optional bool nodep = 12;
   */
  nodep: boolean;

  /**
   * @generated from field: optional bool cfg_is_host = 13;
   */
  cfgIsHost: boolean;
};

/**
 * Describes the message blaze_query_aspect_mirror.AttributeDefinition.
 * Use `create(AttributeDefinitionSchema)` to create a new message.
 */
export declare const AttributeDefinitionSchema: GenMessage<AttributeDefinition>;

/**
 * @generated from message blaze_query_aspect_mirror.AttributeValue
 */
export declare type AttributeValue = Message<"blaze_query_aspect_mirror.AttributeValue"> & {
  /**
   * @generated from field: optional int32 int = 1;
   */
  int: number;

  /**
   * @generated from field: optional string string = 2;
   */
  string: string;

  /**
   * @generated from field: optional bool bool = 3;
   */
  bool: boolean;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.AttributeValue list = 4;
   */
  list: AttributeValue[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.AttributeValue.DictEntry dict = 5;
   */
  dict: AttributeValue_DictEntry[];
};

/**
 * Describes the message blaze_query_aspect_mirror.AttributeValue.
 * Use `create(AttributeValueSchema)` to create a new message.
 */
export declare const AttributeValueSchema: GenMessage<AttributeValue>;

/**
 * @generated from message blaze_query_aspect_mirror.AttributeValue.DictEntry
 */
export declare type AttributeValue_DictEntry = Message<"blaze_query_aspect_mirror.AttributeValue.DictEntry"> & {
  /**
   * @generated from field: required string key = 1;
   */
  key: string;

  /**
   * @generated from field: required blaze_query_aspect_mirror.AttributeValue value = 2;
   */
  value?: AttributeValue;
};

/**
 * Describes the message blaze_query_aspect_mirror.AttributeValue.DictEntry.
 * Use `create(AttributeValue_DictEntrySchema)` to create a new message.
 */
export declare const AttributeValue_DictEntrySchema: GenMessage<AttributeValue_DictEntry>;

/**
 * @generated from message blaze_query_aspect_mirror.RuleDefinition
 */
export declare type RuleDefinition = Message<"blaze_query_aspect_mirror.RuleDefinition"> & {
  /**
   * @generated from field: required string name = 1;
   */
  name: string;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.AttributeDefinition attribute = 2;
   */
  attribute: AttributeDefinition[];

  /**
   * @generated from field: optional string documentation = 3;
   */
  documentation: string;

  /**
   * @generated from field: optional string label = 4;
   */
  label: string;
};

/**
 * Describes the message blaze_query_aspect_mirror.RuleDefinition.
 * Use `create(RuleDefinitionSchema)` to create a new message.
 */
export declare const RuleDefinitionSchema: GenMessage<RuleDefinition>;

/**
 * @generated from message blaze_query_aspect_mirror.BuildLanguage
 */
export declare type BuildLanguage = Message<"blaze_query_aspect_mirror.BuildLanguage"> & {
  /**
   * @generated from field: repeated blaze_query_aspect_mirror.RuleDefinition rule = 1;
   */
  rule: RuleDefinition[];
};

/**
 * Describes the message blaze_query_aspect_mirror.BuildLanguage.
 * Use `create(BuildLanguageSchema)` to create a new message.
 */
export declare const BuildLanguageSchema: GenMessage<BuildLanguage>;

