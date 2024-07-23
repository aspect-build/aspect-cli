// @generated by protoc-gen-es v1.8.0 with parameter "keep_empty_files=true,target=js+dts,js_import_style=legacy_commonjs"
// @generated from file bazel/invocation_policy/invocation_policy.proto (package invocation_policy, syntax proto2)
/* eslint-disable */
// @ts-nocheck

import type { BinaryReadOptions, FieldList, JsonReadOptions, JsonValue, PartialMessage, PlainMessage } from "@bufbuild/protobuf";
import { Message, proto2 } from "@bufbuild/protobuf";

/**
 * @generated from message invocation_policy.InvocationPolicy
 */
export declare class InvocationPolicy extends Message<InvocationPolicy> {
  /**
   * @generated from field: repeated invocation_policy.FlagPolicy flag_policies = 1;
   */
  flagPolicies: FlagPolicy[];

  constructor(data?: PartialMessage<InvocationPolicy>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "invocation_policy.InvocationPolicy";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): InvocationPolicy;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): InvocationPolicy;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): InvocationPolicy;

  static equals(a: InvocationPolicy | PlainMessage<InvocationPolicy> | undefined, b: InvocationPolicy | PlainMessage<InvocationPolicy> | undefined): boolean;
}

/**
 * @generated from message invocation_policy.FlagPolicy
 */
export declare class FlagPolicy extends Message<FlagPolicy> {
  /**
   * @generated from field: optional string flag_name = 1;
   */
  flagName?: string;

  /**
   * @generated from field: repeated string commands = 2;
   */
  commands: string[];

  /**
   * @generated from oneof invocation_policy.FlagPolicy.operation
   */
  operation: {
    /**
     * @generated from field: invocation_policy.SetValue set_value = 3;
     */
    value: SetValue;
    case: "setValue";
  } | {
    /**
     * @generated from field: invocation_policy.UseDefault use_default = 4;
     */
    value: UseDefault;
    case: "useDefault";
  } | {
    /**
     * @generated from field: invocation_policy.DisallowValues disallow_values = 5;
     */
    value: DisallowValues;
    case: "disallowValues";
  } | {
    /**
     * @generated from field: invocation_policy.AllowValues allow_values = 6;
     */
    value: AllowValues;
    case: "allowValues";
  } | { case: undefined; value?: undefined };

  constructor(data?: PartialMessage<FlagPolicy>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "invocation_policy.FlagPolicy";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): FlagPolicy;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): FlagPolicy;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): FlagPolicy;

  static equals(a: FlagPolicy | PlainMessage<FlagPolicy> | undefined, b: FlagPolicy | PlainMessage<FlagPolicy> | undefined): boolean;
}

/**
 * @generated from message invocation_policy.SetValue
 */
export declare class SetValue extends Message<SetValue> {
  /**
   * @generated from field: repeated string flag_value = 1;
   */
  flagValue: string[];

  /**
   * @generated from field: optional invocation_policy.SetValue.Behavior behavior = 4;
   */
  behavior?: SetValue_Behavior;

  constructor(data?: PartialMessage<SetValue>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "invocation_policy.SetValue";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): SetValue;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): SetValue;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): SetValue;

  static equals(a: SetValue | PlainMessage<SetValue> | undefined, b: SetValue | PlainMessage<SetValue> | undefined): boolean;
}

/**
 * @generated from enum invocation_policy.SetValue.Behavior
 */
export declare enum SetValue_Behavior {
  /**
   * @generated from enum value: UNDEFINED = 0;
   */
  UNDEFINED = 0,

  /**
   * @generated from enum value: ALLOW_OVERRIDES = 1;
   */
  ALLOW_OVERRIDES = 1,

  /**
   * @generated from enum value: APPEND = 2;
   */
  APPEND = 2,

  /**
   * @generated from enum value: FINAL_VALUE_IGNORE_OVERRIDES = 3;
   */
  FINAL_VALUE_IGNORE_OVERRIDES = 3,
}

/**
 * @generated from message invocation_policy.UseDefault
 */
export declare class UseDefault extends Message<UseDefault> {
  constructor(data?: PartialMessage<UseDefault>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "invocation_policy.UseDefault";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): UseDefault;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): UseDefault;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): UseDefault;

  static equals(a: UseDefault | PlainMessage<UseDefault> | undefined, b: UseDefault | PlainMessage<UseDefault> | undefined): boolean;
}

/**
 * @generated from message invocation_policy.DisallowValues
 */
export declare class DisallowValues extends Message<DisallowValues> {
  /**
   * @generated from field: repeated string disallowed_values = 1;
   */
  disallowedValues: string[];

  /**
   * @generated from oneof invocation_policy.DisallowValues.replacement_value
   */
  replacementValue: {
    /**
     * @generated from field: string new_value = 3;
     */
    value: string;
    case: "newValue";
  } | {
    /**
     * @generated from field: invocation_policy.UseDefault use_default = 4;
     */
    value: UseDefault;
    case: "useDefault";
  } | { case: undefined; value?: undefined };

  constructor(data?: PartialMessage<DisallowValues>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "invocation_policy.DisallowValues";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): DisallowValues;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): DisallowValues;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): DisallowValues;

  static equals(a: DisallowValues | PlainMessage<DisallowValues> | undefined, b: DisallowValues | PlainMessage<DisallowValues> | undefined): boolean;
}

/**
 * @generated from message invocation_policy.AllowValues
 */
export declare class AllowValues extends Message<AllowValues> {
  /**
   * @generated from field: repeated string allowed_values = 1;
   */
  allowedValues: string[];

  /**
   * @generated from oneof invocation_policy.AllowValues.replacement_value
   */
  replacementValue: {
    /**
     * @generated from field: string new_value = 3;
     */
    value: string;
    case: "newValue";
  } | {
    /**
     * @generated from field: invocation_policy.UseDefault use_default = 4;
     */
    value: UseDefault;
    case: "useDefault";
  } | { case: undefined; value?: undefined };

  constructor(data?: PartialMessage<AllowValues>);

  static readonly runtime: typeof proto2;
  static readonly typeName = "invocation_policy.AllowValues";
  static readonly fields: FieldList;

  static fromBinary(bytes: Uint8Array, options?: Partial<BinaryReadOptions>): AllowValues;

  static fromJson(jsonValue: JsonValue, options?: Partial<JsonReadOptions>): AllowValues;

  static fromJsonString(jsonString: string, options?: Partial<JsonReadOptions>): AllowValues;

  static equals(a: AllowValues | PlainMessage<AllowValues> | undefined, b: AllowValues | PlainMessage<AllowValues> | undefined): boolean;
}

