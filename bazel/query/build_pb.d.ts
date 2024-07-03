// @generated by protoc-gen-es v1.8.0 with parameter "keep_empty_files=true,target=js+dts"
// @generated from file cli/core/bazel/query/build.proto (package blaze_query_aspect_mirror, syntax proto2)
/* eslint-disable */
// @ts-nocheck

import type { BinaryReadOptions, FieldList, JsonReadOptions, JsonValue, PartialMessage, PlainMessage } from "@bufbuild/protobuf";
import { Message, proto2 } from "@bufbuild/protobuf";

/**
 * @generated from message blaze_query_aspect_mirror.License
 */
export declare class License extends Message<License> {
  /**
   * @generated from field: repeated string license_type = 1;
   */
  licenseType: string[];

  /**
   * @generated from field: repeated string exception = 2;
   */
  exception: string[];

  constructor(data?: PartialMessage<License>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.License";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): License;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): License;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): License;

  static equals(a: License | PlainMessage<License> | undefined, b: License | PlainMessage<License> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.StringDictEntry
 */
export declare class StringDictEntry extends Message<StringDictEntry> {
  /**
   * @generated from field: required string key = 1;
   */
  key?: string;

  /**
   * @generated from field: required string value = 2;
   */
  value?: string;

  constructor(data?: PartialMessage<StringDictEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.StringDictEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): StringDictEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): StringDictEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): StringDictEntry;

  static equals(a: StringDictEntry | PlainMessage<StringDictEntry> | undefined, b: StringDictEntry | PlainMessage<StringDictEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.LabelDictUnaryEntry
 */
export declare class LabelDictUnaryEntry extends Message<LabelDictUnaryEntry> {
  /**
   * @generated from field: required string key = 1;
   */
  key?: string;

  /**
   * @generated from field: required string value = 2;
   */
  value?: string;

  constructor(data?: PartialMessage<LabelDictUnaryEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.LabelDictUnaryEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): LabelDictUnaryEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): LabelDictUnaryEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): LabelDictUnaryEntry;

  static equals(a: LabelDictUnaryEntry | PlainMessage<LabelDictUnaryEntry> | undefined, b: LabelDictUnaryEntry | PlainMessage<LabelDictUnaryEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.LabelListDictEntry
 */
export declare class LabelListDictEntry extends Message<LabelListDictEntry> {
  /**
   * @generated from field: required string key = 1;
   */
  key?: string;

  /**
   * @generated from field: repeated string value = 2;
   */
  value: string[];

  constructor(data?: PartialMessage<LabelListDictEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.LabelListDictEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): LabelListDictEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): LabelListDictEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): LabelListDictEntry;

  static equals(a: LabelListDictEntry | PlainMessage<LabelListDictEntry> | undefined, b: LabelListDictEntry | PlainMessage<LabelListDictEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.LabelKeyedStringDictEntry
 */
export declare class LabelKeyedStringDictEntry extends Message<LabelKeyedStringDictEntry> {
  /**
   * @generated from field: required string key = 1;
   */
  key?: string;

  /**
   * @generated from field: required string value = 2;
   */
  value?: string;

  constructor(data?: PartialMessage<LabelKeyedStringDictEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.LabelKeyedStringDictEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): LabelKeyedStringDictEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): LabelKeyedStringDictEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): LabelKeyedStringDictEntry;

  static equals(a: LabelKeyedStringDictEntry | PlainMessage<LabelKeyedStringDictEntry> | undefined, b: LabelKeyedStringDictEntry | PlainMessage<LabelKeyedStringDictEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.StringListDictEntry
 */
export declare class StringListDictEntry extends Message<StringListDictEntry> {
  /**
   * @generated from field: required string key = 1;
   */
  key?: string;

  /**
   * @generated from field: repeated string value = 2;
   */
  value: string[];

  constructor(data?: PartialMessage<StringListDictEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.StringListDictEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): StringListDictEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): StringListDictEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): StringListDictEntry;

  static equals(a: StringListDictEntry | PlainMessage<StringListDictEntry> | undefined, b: StringListDictEntry | PlainMessage<StringListDictEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.FilesetEntry
 */
export declare class FilesetEntry extends Message<FilesetEntry> {
  /**
   * @generated from field: required string source = 1;
   */
  source?: string;

  /**
   * @generated from field: required string destination_directory = 2;
   */
  destinationDirectory?: string;

  /**
   * @generated from field: optional bool files_present = 7;
   */
  filesPresent?: boolean;

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
  symlinkBehavior?: FilesetEntry_SymlinkBehavior;

  /**
   * @generated from field: optional string strip_prefix = 6;
   */
  stripPrefix?: string;

  constructor(data?: PartialMessage<FilesetEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.FilesetEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): FilesetEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): FilesetEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): FilesetEntry;

  static equals(a: FilesetEntry | PlainMessage<FilesetEntry> | undefined, b: FilesetEntry | PlainMessage<FilesetEntry> | undefined): boolean;
}

/**
 * @generated from enum blaze_query_aspect_mirror.FilesetEntry.SymlinkBehavior
 */
export declare enum FilesetEntry_SymlinkBehavior {
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
 * @generated from message blaze_query_aspect_mirror.Attribute
 */
export declare class Attribute extends Message<Attribute> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: optional bool explicitly_specified = 13;
   */
  explicitlySpecified?: boolean;

  /**
   * @generated from field: optional bool nodep = 20;
   */
  nodep?: boolean;

  /**
   * @generated from field: optional string source_aspect_name = 23;
   */
  sourceAspectName?: string;

  /**
   * @generated from field: required blaze_query_aspect_mirror.Attribute.Discriminator type = 2;
   */
  type?: Attribute_Discriminator;

  /**
   * @generated from field: optional int32 int_value = 3;
   */
  intValue?: number;

  /**
   * @generated from field: optional string string_value = 5;
   */
  stringValue?: string;

  /**
   * @generated from field: optional bool boolean_value = 14;
   */
  booleanValue?: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.Tristate tristate_value = 15;
   */
  tristateValue?: Attribute_Tristate;

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

  constructor(data?: PartialMessage<Attribute>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.Attribute";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): Attribute;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): Attribute;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): Attribute;

  static equals(a: Attribute | PlainMessage<Attribute> | undefined, b: Attribute | PlainMessage<Attribute> | undefined): boolean;
}

/**
 * @generated from enum blaze_query_aspect_mirror.Attribute.Discriminator
 */
export declare enum Attribute_Discriminator {
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
 * @generated from enum blaze_query_aspect_mirror.Attribute.Tristate
 */
export declare enum Attribute_Tristate {
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
 * @generated from message blaze_query_aspect_mirror.Attribute.SelectorEntry
 */
export declare class Attribute_SelectorEntry extends Message<Attribute_SelectorEntry> {
  /**
   * @generated from field: optional string label = 1;
   */
  label?: string;

  /**
   * @generated from field: optional bool is_default_value = 16;
   */
  isDefaultValue?: boolean;

  /**
   * @generated from field: optional int32 int_value = 2;
   */
  intValue?: number;

  /**
   * @generated from field: optional string string_value = 3;
   */
  stringValue?: string;

  /**
   * @generated from field: optional bool boolean_value = 4;
   */
  booleanValue?: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.Tristate tristate_value = 5;
   */
  tristateValue?: Attribute_Tristate;

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

  constructor(data?: PartialMessage<Attribute_SelectorEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.Attribute.SelectorEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): Attribute_SelectorEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): Attribute_SelectorEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): Attribute_SelectorEntry;

  static equals(a: Attribute_SelectorEntry | PlainMessage<Attribute_SelectorEntry> | undefined, b: Attribute_SelectorEntry | PlainMessage<Attribute_SelectorEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.Attribute.Selector
 */
export declare class Attribute_Selector extends Message<Attribute_Selector> {
  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Attribute.SelectorEntry entries = 1;
   */
  entries: Attribute_SelectorEntry[];

  /**
   * @generated from field: optional bool has_default_value = 2;
   */
  hasDefaultValue?: boolean;

  /**
   * @generated from field: optional string no_match_error = 3;
   */
  noMatchError?: string;

  constructor(data?: PartialMessage<Attribute_Selector>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.Attribute.Selector";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): Attribute_Selector;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): Attribute_Selector;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): Attribute_Selector;

  static equals(a: Attribute_Selector | PlainMessage<Attribute_Selector> | undefined, b: Attribute_Selector | PlainMessage<Attribute_Selector> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.Attribute.SelectorList
 */
export declare class Attribute_SelectorList extends Message<Attribute_SelectorList> {
  /**
   * @generated from field: optional blaze_query_aspect_mirror.Attribute.Discriminator type = 1;
   */
  type?: Attribute_Discriminator;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Attribute.Selector elements = 2;
   */
  elements: Attribute_Selector[];

  constructor(data?: PartialMessage<Attribute_SelectorList>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.Attribute.SelectorList";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): Attribute_SelectorList;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): Attribute_SelectorList;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): Attribute_SelectorList;

  static equals(a: Attribute_SelectorList | PlainMessage<Attribute_SelectorList> | undefined, b: Attribute_SelectorList | PlainMessage<Attribute_SelectorList> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.Rule
 */
export declare class Rule extends Message<Rule> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: required string rule_class = 2;
   */
  ruleClass?: string;

  /**
   * @generated from field: optional string location = 3;
   */
  location?: string;

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
  DEPRECATEDPublicByDefault?: boolean;

  /**
   * @generated from field: optional bool DEPRECATED_is_skylark = 10;
   */
  DEPRECATEDIsSkylark?: boolean;

  /**
   * @generated from field: optional string skylark_environment_hash_code = 12;
   */
  skylarkEnvironmentHashCode?: string;

  /**
   * @generated from field: repeated string instantiation_stack = 13;
   */
  instantiationStack: string[];

  /**
   * @generated from field: repeated string definition_stack = 14;
   */
  definitionStack: string[];

  constructor(data?: PartialMessage<Rule>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.Rule";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): Rule;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): Rule;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): Rule;

  static equals(a: Rule | PlainMessage<Rule> | undefined, b: Rule | PlainMessage<Rule> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.ConfiguredRuleInput
 */
export declare class ConfiguredRuleInput extends Message<ConfiguredRuleInput> {
  /**
   * @generated from field: optional string label = 1;
   */
  label?: string;

  /**
   * @generated from field: optional string configuration_checksum = 2;
   */
  configurationChecksum?: string;

  /**
   * @generated from field: optional uint32 configuration_id = 3;
   */
  configurationId?: number;

  constructor(data?: PartialMessage<ConfiguredRuleInput>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.ConfiguredRuleInput";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): ConfiguredRuleInput;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): ConfiguredRuleInput;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): ConfiguredRuleInput;

  static equals(a: ConfiguredRuleInput | PlainMessage<ConfiguredRuleInput> | undefined, b: ConfiguredRuleInput | PlainMessage<ConfiguredRuleInput> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.RuleSummary
 */
export declare class RuleSummary extends Message<RuleSummary> {
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
  location?: string;

  constructor(data?: PartialMessage<RuleSummary>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.RuleSummary";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): RuleSummary;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): RuleSummary;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): RuleSummary;

  static equals(a: RuleSummary | PlainMessage<RuleSummary> | undefined, b: RuleSummary | PlainMessage<RuleSummary> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.PackageGroup
 */
export declare class PackageGroup extends Message<PackageGroup> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: repeated string contained_package = 2;
   */
  containedPackage: string[];

  /**
   * @generated from field: repeated string included_package_group = 3;
   */
  includedPackageGroup: string[];

  constructor(data?: PartialMessage<PackageGroup>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.PackageGroup";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): PackageGroup;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): PackageGroup;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): PackageGroup;

  static equals(a: PackageGroup | PlainMessage<PackageGroup> | undefined, b: PackageGroup | PlainMessage<PackageGroup> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.EnvironmentGroup
 */
export declare class EnvironmentGroup extends Message<EnvironmentGroup> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: repeated string environment = 2;
   */
  environment: string[];

  /**
   * @generated from field: repeated string default = 3;
   */
  default: string[];

  constructor(data?: PartialMessage<EnvironmentGroup>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.EnvironmentGroup";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): EnvironmentGroup;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): EnvironmentGroup;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): EnvironmentGroup;

  static equals(a: EnvironmentGroup | PlainMessage<EnvironmentGroup> | undefined, b: EnvironmentGroup | PlainMessage<EnvironmentGroup> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.SourceFile
 */
export declare class SourceFile extends Message<SourceFile> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: optional string location = 2;
   */
  location?: string;

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
  packageContainsErrors?: boolean;

  constructor(data?: PartialMessage<SourceFile>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.SourceFile";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): SourceFile;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): SourceFile;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): SourceFile;

  static equals(a: SourceFile | PlainMessage<SourceFile> | undefined, b: SourceFile | PlainMessage<SourceFile> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.GeneratedFile
 */
export declare class GeneratedFile extends Message<GeneratedFile> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: required string generating_rule = 2;
   */
  generatingRule?: string;

  /**
   * @generated from field: optional string location = 3;
   */
  location?: string;

  constructor(data?: PartialMessage<GeneratedFile>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.GeneratedFile";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): GeneratedFile;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): GeneratedFile;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): GeneratedFile;

  static equals(a: GeneratedFile | PlainMessage<GeneratedFile> | undefined, b: GeneratedFile | PlainMessage<GeneratedFile> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.Target
 */
export declare class Target extends Message<Target> {
  /**
   * @generated from field: required blaze_query_aspect_mirror.Target.Discriminator type = 1;
   */
  type?: Target_Discriminator;

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

  constructor(data?: PartialMessage<Target>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.Target";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): Target;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): Target;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): Target;

  static equals(a: Target | PlainMessage<Target> | undefined, b: Target | PlainMessage<Target> | undefined): boolean;
}

/**
 * @generated from enum blaze_query_aspect_mirror.Target.Discriminator
 */
export declare enum Target_Discriminator {
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
 * @generated from message blaze_query_aspect_mirror.QueryResult
 */
export declare class QueryResult extends Message<QueryResult> {
  /**
   * @generated from field: repeated blaze_query_aspect_mirror.Target target = 1;
   */
  target: Target[];

  constructor(data?: PartialMessage<QueryResult>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.QueryResult";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): QueryResult;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): QueryResult;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): QueryResult;

  static equals(a: QueryResult | PlainMessage<QueryResult> | undefined, b: QueryResult | PlainMessage<QueryResult> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.AllowedRuleClassInfo
 */
export declare class AllowedRuleClassInfo extends Message<AllowedRuleClassInfo> {
  /**
   * @generated from field: required blaze_query_aspect_mirror.AllowedRuleClassInfo.AllowedRuleClasses policy = 1;
   */
  policy?: AllowedRuleClassInfo_AllowedRuleClasses;

  /**
   * @generated from field: repeated string allowed_rule_class = 2;
   */
  allowedRuleClass: string[];

  constructor(data?: PartialMessage<AllowedRuleClassInfo>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.AllowedRuleClassInfo";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): AllowedRuleClassInfo;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): AllowedRuleClassInfo;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): AllowedRuleClassInfo;

  static equals(a: AllowedRuleClassInfo | PlainMessage<AllowedRuleClassInfo> | undefined, b: AllowedRuleClassInfo | PlainMessage<AllowedRuleClassInfo> | undefined): boolean;
}

/**
 * @generated from enum blaze_query_aspect_mirror.AllowedRuleClassInfo.AllowedRuleClasses
 */
export declare enum AllowedRuleClassInfo_AllowedRuleClasses {
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
 * @generated from message blaze_query_aspect_mirror.AttributeDefinition
 */
export declare class AttributeDefinition extends Message<AttributeDefinition> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: required blaze_query_aspect_mirror.Attribute.Discriminator type = 2;
   */
  type?: Attribute_Discriminator;

  /**
   * @generated from field: optional bool mandatory = 3;
   */
  mandatory?: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.AllowedRuleClassInfo allowed_rule_classes = 4;
   */
  allowedRuleClasses?: AllowedRuleClassInfo;

  /**
   * @generated from field: optional string documentation = 5;
   */
  documentation?: string;

  /**
   * @generated from field: optional bool allow_empty = 6;
   */
  allowEmpty?: boolean;

  /**
   * @generated from field: optional bool allow_single_file = 7;
   */
  allowSingleFile?: boolean;

  /**
   * @generated from field: optional blaze_query_aspect_mirror.AttributeValue default = 9;
   */
  default?: AttributeValue;

  /**
   * @generated from field: optional bool executable = 10;
   */
  executable?: boolean;

  /**
   * @generated from field: optional bool configurable = 11;
   */
  configurable?: boolean;

  /**
   * @generated from field: optional bool nodep = 12;
   */
  nodep?: boolean;

  /**
   * @generated from field: optional bool cfg_is_host = 13;
   */
  cfgIsHost?: boolean;

  constructor(data?: PartialMessage<AttributeDefinition>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.AttributeDefinition";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): AttributeDefinition;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): AttributeDefinition;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): AttributeDefinition;

  static equals(a: AttributeDefinition | PlainMessage<AttributeDefinition> | undefined, b: AttributeDefinition | PlainMessage<AttributeDefinition> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.AttributeValue
 */
export declare class AttributeValue extends Message<AttributeValue> {
  /**
   * @generated from field: optional int32 int = 1;
   */
  int?: number;

  /**
   * @generated from field: optional string string = 2;
   */
  string?: string;

  /**
   * @generated from field: optional bool bool = 3;
   */
  bool?: boolean;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.AttributeValue list = 4;
   */
  list: AttributeValue[];

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.AttributeValue.DictEntry dict = 5;
   */
  dict: AttributeValue_DictEntry[];

  constructor(data?: PartialMessage<AttributeValue>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.AttributeValue";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): AttributeValue;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): AttributeValue;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): AttributeValue;

  static equals(a: AttributeValue | PlainMessage<AttributeValue> | undefined, b: AttributeValue | PlainMessage<AttributeValue> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.AttributeValue.DictEntry
 */
export declare class AttributeValue_DictEntry extends Message<AttributeValue_DictEntry> {
  /**
   * @generated from field: required string key = 1;
   */
  key?: string;

  /**
   * @generated from field: required blaze_query_aspect_mirror.AttributeValue value = 2;
   */
  value?: AttributeValue;

  constructor(data?: PartialMessage<AttributeValue_DictEntry>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.AttributeValue.DictEntry";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): AttributeValue_DictEntry;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): AttributeValue_DictEntry;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): AttributeValue_DictEntry;

  static equals(a: AttributeValue_DictEntry | PlainMessage<AttributeValue_DictEntry> | undefined, b: AttributeValue_DictEntry | PlainMessage<AttributeValue_DictEntry> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.RuleDefinition
 */
export declare class RuleDefinition extends Message<RuleDefinition> {
  /**
   * @generated from field: required string name = 1;
   */
  name?: string;

  /**
   * @generated from field: repeated blaze_query_aspect_mirror.AttributeDefinition attribute = 2;
   */
  attribute: AttributeDefinition[];

  /**
   * @generated from field: optional string documentation = 3;
   */
  documentation?: string;

  /**
   * @generated from field: optional string label = 4;
   */
  label?: string;

  constructor(data?: PartialMessage<RuleDefinition>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.RuleDefinition";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): RuleDefinition;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): RuleDefinition;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): RuleDefinition;

  static equals(a: RuleDefinition | PlainMessage<RuleDefinition> | undefined, b: RuleDefinition | PlainMessage<RuleDefinition> | undefined): boolean;
}

/**
 * @generated from message blaze_query_aspect_mirror.BuildLanguage
 */
export declare class BuildLanguage extends Message<BuildLanguage> {
  /**
   * @generated from field: repeated blaze_query_aspect_mirror.RuleDefinition rule = 1;
   */
  rule: RuleDefinition[];

  constructor(data?: PartialMessage<BuildLanguage>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "blaze_query_aspect_mirror.BuildLanguage";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): BuildLanguage;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): BuildLanguage;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): BuildLanguage;

  static equals(a: BuildLanguage | PlainMessage<BuildLanguage> | undefined, b: BuildLanguage | PlainMessage<BuildLanguage> | undefined): boolean;
}
