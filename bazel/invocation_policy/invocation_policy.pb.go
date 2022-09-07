// Code generated by protoc-gen-go. DO NOT EDIT.
// versions:
// 	protoc-gen-go v1.28.0
// 	protoc        v3.14.0
// source: bazel/invocation_policy/invocation_policy.proto

package invocation_policy

import (
	protoreflect "google.golang.org/protobuf/reflect/protoreflect"
	protoimpl "google.golang.org/protobuf/runtime/protoimpl"
	reflect "reflect"
	sync "sync"
)

const (
	// Verify that this generated code is sufficiently up-to-date.
	_ = protoimpl.EnforceVersion(20 - protoimpl.MinVersion)
	// Verify that runtime/protoimpl is sufficiently up-to-date.
	_ = protoimpl.EnforceVersion(protoimpl.MaxVersion - 20)
)

type InvocationPolicy struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	FlagPolicies []*FlagPolicy `protobuf:"bytes,1,rep,name=flag_policies,json=flagPolicies" json:"flag_policies,omitempty"`
}

func (x *InvocationPolicy) Reset() {
	*x = InvocationPolicy{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[0]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *InvocationPolicy) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*InvocationPolicy) ProtoMessage() {}

func (x *InvocationPolicy) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[0]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use InvocationPolicy.ProtoReflect.Descriptor instead.
func (*InvocationPolicy) Descriptor() ([]byte, []int) {
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP(), []int{0}
}

func (x *InvocationPolicy) GetFlagPolicies() []*FlagPolicy {
	if x != nil {
		return x.FlagPolicies
	}
	return nil
}

type FlagPolicy struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	FlagName *string  `protobuf:"bytes,1,opt,name=flag_name,json=flagName" json:"flag_name,omitempty"`
	Commands []string `protobuf:"bytes,2,rep,name=commands" json:"commands,omitempty"`
	// Types that are assignable to Operation:
	//
	//	*FlagPolicy_SetValue
	//	*FlagPolicy_UseDefault
	//	*FlagPolicy_DisallowValues
	//	*FlagPolicy_AllowValues
	Operation isFlagPolicy_Operation `protobuf_oneof:"operation"`
}

func (x *FlagPolicy) Reset() {
	*x = FlagPolicy{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[1]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *FlagPolicy) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*FlagPolicy) ProtoMessage() {}

func (x *FlagPolicy) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[1]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use FlagPolicy.ProtoReflect.Descriptor instead.
func (*FlagPolicy) Descriptor() ([]byte, []int) {
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP(), []int{1}
}

func (x *FlagPolicy) GetFlagName() string {
	if x != nil && x.FlagName != nil {
		return *x.FlagName
	}
	return ""
}

func (x *FlagPolicy) GetCommands() []string {
	if x != nil {
		return x.Commands
	}
	return nil
}

func (m *FlagPolicy) GetOperation() isFlagPolicy_Operation {
	if m != nil {
		return m.Operation
	}
	return nil
}

func (x *FlagPolicy) GetSetValue() *SetValue {
	if x, ok := x.GetOperation().(*FlagPolicy_SetValue); ok {
		return x.SetValue
	}
	return nil
}

func (x *FlagPolicy) GetUseDefault() *UseDefault {
	if x, ok := x.GetOperation().(*FlagPolicy_UseDefault); ok {
		return x.UseDefault
	}
	return nil
}

func (x *FlagPolicy) GetDisallowValues() *DisallowValues {
	if x, ok := x.GetOperation().(*FlagPolicy_DisallowValues); ok {
		return x.DisallowValues
	}
	return nil
}

func (x *FlagPolicy) GetAllowValues() *AllowValues {
	if x, ok := x.GetOperation().(*FlagPolicy_AllowValues); ok {
		return x.AllowValues
	}
	return nil
}

type isFlagPolicy_Operation interface {
	isFlagPolicy_Operation()
}

type FlagPolicy_SetValue struct {
	SetValue *SetValue `protobuf:"bytes,3,opt,name=set_value,json=setValue,oneof"`
}

type FlagPolicy_UseDefault struct {
	UseDefault *UseDefault `protobuf:"bytes,4,opt,name=use_default,json=useDefault,oneof"`
}

type FlagPolicy_DisallowValues struct {
	DisallowValues *DisallowValues `protobuf:"bytes,5,opt,name=disallow_values,json=disallowValues,oneof"`
}

type FlagPolicy_AllowValues struct {
	AllowValues *AllowValues `protobuf:"bytes,6,opt,name=allow_values,json=allowValues,oneof"`
}

func (*FlagPolicy_SetValue) isFlagPolicy_Operation() {}

func (*FlagPolicy_UseDefault) isFlagPolicy_Operation() {}

func (*FlagPolicy_DisallowValues) isFlagPolicy_Operation() {}

func (*FlagPolicy_AllowValues) isFlagPolicy_Operation() {}

type SetValue struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	FlagValue   []string `protobuf:"bytes,1,rep,name=flag_value,json=flagValue" json:"flag_value,omitempty"`
	Overridable *bool    `protobuf:"varint,2,opt,name=overridable" json:"overridable,omitempty"`
	Append      *bool    `protobuf:"varint,3,opt,name=append" json:"append,omitempty"`
}

func (x *SetValue) Reset() {
	*x = SetValue{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[2]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *SetValue) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*SetValue) ProtoMessage() {}

func (x *SetValue) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[2]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use SetValue.ProtoReflect.Descriptor instead.
func (*SetValue) Descriptor() ([]byte, []int) {
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP(), []int{2}
}

func (x *SetValue) GetFlagValue() []string {
	if x != nil {
		return x.FlagValue
	}
	return nil
}

func (x *SetValue) GetOverridable() bool {
	if x != nil && x.Overridable != nil {
		return *x.Overridable
	}
	return false
}

func (x *SetValue) GetAppend() bool {
	if x != nil && x.Append != nil {
		return *x.Append
	}
	return false
}

type UseDefault struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields
}

func (x *UseDefault) Reset() {
	*x = UseDefault{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[3]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *UseDefault) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*UseDefault) ProtoMessage() {}

func (x *UseDefault) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[3]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use UseDefault.ProtoReflect.Descriptor instead.
func (*UseDefault) Descriptor() ([]byte, []int) {
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP(), []int{3}
}

type DisallowValues struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	DisallowedValues []string `protobuf:"bytes,1,rep,name=disallowed_values,json=disallowedValues" json:"disallowed_values,omitempty"`
	// Types that are assignable to ReplacementValue:
	//
	//	*DisallowValues_NewValue
	//	*DisallowValues_UseDefault
	ReplacementValue isDisallowValues_ReplacementValue `protobuf_oneof:"replacement_value"`
}

func (x *DisallowValues) Reset() {
	*x = DisallowValues{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[4]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *DisallowValues) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*DisallowValues) ProtoMessage() {}

func (x *DisallowValues) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[4]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use DisallowValues.ProtoReflect.Descriptor instead.
func (*DisallowValues) Descriptor() ([]byte, []int) {
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP(), []int{4}
}

func (x *DisallowValues) GetDisallowedValues() []string {
	if x != nil {
		return x.DisallowedValues
	}
	return nil
}

func (m *DisallowValues) GetReplacementValue() isDisallowValues_ReplacementValue {
	if m != nil {
		return m.ReplacementValue
	}
	return nil
}

func (x *DisallowValues) GetNewValue() string {
	if x, ok := x.GetReplacementValue().(*DisallowValues_NewValue); ok {
		return x.NewValue
	}
	return ""
}

func (x *DisallowValues) GetUseDefault() *UseDefault {
	if x, ok := x.GetReplacementValue().(*DisallowValues_UseDefault); ok {
		return x.UseDefault
	}
	return nil
}

type isDisallowValues_ReplacementValue interface {
	isDisallowValues_ReplacementValue()
}

type DisallowValues_NewValue struct {
	NewValue string `protobuf:"bytes,3,opt,name=new_value,json=newValue,oneof"`
}

type DisallowValues_UseDefault struct {
	UseDefault *UseDefault `protobuf:"bytes,4,opt,name=use_default,json=useDefault,oneof"`
}

func (*DisallowValues_NewValue) isDisallowValues_ReplacementValue() {}

func (*DisallowValues_UseDefault) isDisallowValues_ReplacementValue() {}

type AllowValues struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	AllowedValues []string `protobuf:"bytes,1,rep,name=allowed_values,json=allowedValues" json:"allowed_values,omitempty"`
	// Types that are assignable to ReplacementValue:
	//
	//	*AllowValues_NewValue
	//	*AllowValues_UseDefault
	ReplacementValue isAllowValues_ReplacementValue `protobuf_oneof:"replacement_value"`
}

func (x *AllowValues) Reset() {
	*x = AllowValues{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[5]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *AllowValues) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*AllowValues) ProtoMessage() {}

func (x *AllowValues) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_invocation_policy_invocation_policy_proto_msgTypes[5]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use AllowValues.ProtoReflect.Descriptor instead.
func (*AllowValues) Descriptor() ([]byte, []int) {
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP(), []int{5}
}

func (x *AllowValues) GetAllowedValues() []string {
	if x != nil {
		return x.AllowedValues
	}
	return nil
}

func (m *AllowValues) GetReplacementValue() isAllowValues_ReplacementValue {
	if m != nil {
		return m.ReplacementValue
	}
	return nil
}

func (x *AllowValues) GetNewValue() string {
	if x, ok := x.GetReplacementValue().(*AllowValues_NewValue); ok {
		return x.NewValue
	}
	return ""
}

func (x *AllowValues) GetUseDefault() *UseDefault {
	if x, ok := x.GetReplacementValue().(*AllowValues_UseDefault); ok {
		return x.UseDefault
	}
	return nil
}

type isAllowValues_ReplacementValue interface {
	isAllowValues_ReplacementValue()
}

type AllowValues_NewValue struct {
	NewValue string `protobuf:"bytes,3,opt,name=new_value,json=newValue,oneof"`
}

type AllowValues_UseDefault struct {
	UseDefault *UseDefault `protobuf:"bytes,4,opt,name=use_default,json=useDefault,oneof"`
}

func (*AllowValues_NewValue) isAllowValues_ReplacementValue() {}

func (*AllowValues_UseDefault) isAllowValues_ReplacementValue() {}

var File_bazel_invocation_policy_invocation_policy_proto protoreflect.FileDescriptor

var file_bazel_invocation_policy_invocation_policy_proto_rawDesc = []byte{
	0x0a, 0x2f, 0x62, 0x61, 0x7a, 0x65, 0x6c, 0x2f, 0x69, 0x6e, 0x76, 0x6f, 0x63, 0x61, 0x74, 0x69,
	0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2f, 0x69, 0x6e, 0x76, 0x6f, 0x63, 0x61,
	0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2e, 0x70, 0x72, 0x6f, 0x74,
	0x6f, 0x12, 0x17, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69, 0x6e, 0x76, 0x6f, 0x63, 0x61, 0x74,
	0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x22, 0x5c, 0x0a, 0x10, 0x49, 0x6e,
	0x76, 0x6f, 0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x50, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x12, 0x48,
	0x0a, 0x0d, 0x66, 0x6c, 0x61, 0x67, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x69, 0x65, 0x73, 0x18,
	0x01, 0x20, 0x03, 0x28, 0x0b, 0x32, 0x23, 0x2e, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69, 0x6e,
	0x76, 0x6f, 0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2e,
	0x46, 0x6c, 0x61, 0x67, 0x50, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x52, 0x0c, 0x66, 0x6c, 0x61, 0x67,
	0x50, 0x6f, 0x6c, 0x69, 0x63, 0x69, 0x65, 0x73, 0x22, 0xfb, 0x02, 0x0a, 0x0a, 0x46, 0x6c, 0x61,
	0x67, 0x50, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x12, 0x1b, 0x0a, 0x09, 0x66, 0x6c, 0x61, 0x67, 0x5f,
	0x6e, 0x61, 0x6d, 0x65, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x08, 0x66, 0x6c, 0x61, 0x67,
	0x4e, 0x61, 0x6d, 0x65, 0x12, 0x1a, 0x0a, 0x08, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x73,
	0x18, 0x02, 0x20, 0x03, 0x28, 0x09, 0x52, 0x08, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x73,
	0x12, 0x40, 0x0a, 0x09, 0x73, 0x65, 0x74, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x18, 0x03, 0x20,
	0x01, 0x28, 0x0b, 0x32, 0x21, 0x2e, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69, 0x6e, 0x76, 0x6f,
	0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2e, 0x53, 0x65,
	0x74, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x48, 0x00, 0x52, 0x08, 0x73, 0x65, 0x74, 0x56, 0x61, 0x6c,
	0x75, 0x65, 0x12, 0x46, 0x0a, 0x0b, 0x75, 0x73, 0x65, 0x5f, 0x64, 0x65, 0x66, 0x61, 0x75, 0x6c,
	0x74, 0x18, 0x04, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x23, 0x2e, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e,
	0x69, 0x6e, 0x76, 0x6f, 0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63,
	0x79, 0x2e, 0x55, 0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x48, 0x00, 0x52, 0x0a,
	0x75, 0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x12, 0x52, 0x0a, 0x0f, 0x64, 0x69,
	0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x18, 0x05, 0x20,
	0x01, 0x28, 0x0b, 0x32, 0x27, 0x2e, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69, 0x6e, 0x76, 0x6f,
	0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2e, 0x44, 0x69,
	0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x48, 0x00, 0x52, 0x0e,
	0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x12, 0x49,
	0x0a, 0x0c, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x18, 0x06,
	0x20, 0x01, 0x28, 0x0b, 0x32, 0x24, 0x2e, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69, 0x6e, 0x76,
	0x6f, 0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2e, 0x41,
	0x6c, 0x6c, 0x6f, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x48, 0x00, 0x52, 0x0b, 0x61, 0x6c,
	0x6c, 0x6f, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x42, 0x0b, 0x0a, 0x09, 0x6f, 0x70, 0x65,
	0x72, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x22, 0x63, 0x0a, 0x08, 0x53, 0x65, 0x74, 0x56, 0x61, 0x6c,
	0x75, 0x65, 0x12, 0x1d, 0x0a, 0x0a, 0x66, 0x6c, 0x61, 0x67, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65,
	0x18, 0x01, 0x20, 0x03, 0x28, 0x09, 0x52, 0x09, 0x66, 0x6c, 0x61, 0x67, 0x56, 0x61, 0x6c, 0x75,
	0x65, 0x12, 0x20, 0x0a, 0x0b, 0x6f, 0x76, 0x65, 0x72, 0x72, 0x69, 0x64, 0x61, 0x62, 0x6c, 0x65,
	0x18, 0x02, 0x20, 0x01, 0x28, 0x08, 0x52, 0x0b, 0x6f, 0x76, 0x65, 0x72, 0x72, 0x69, 0x64, 0x61,
	0x62, 0x6c, 0x65, 0x12, 0x16, 0x0a, 0x06, 0x61, 0x70, 0x70, 0x65, 0x6e, 0x64, 0x18, 0x03, 0x20,
	0x01, 0x28, 0x08, 0x52, 0x06, 0x61, 0x70, 0x70, 0x65, 0x6e, 0x64, 0x22, 0x0c, 0x0a, 0x0a, 0x55,
	0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x22, 0xbf, 0x01, 0x0a, 0x0e, 0x44, 0x69,
	0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x12, 0x2b, 0x0a, 0x11,
	0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65,
	0x73, 0x18, 0x01, 0x20, 0x03, 0x28, 0x09, 0x52, 0x10, 0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f,
	0x77, 0x65, 0x64, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x12, 0x1d, 0x0a, 0x09, 0x6e, 0x65, 0x77,
	0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x18, 0x03, 0x20, 0x01, 0x28, 0x09, 0x48, 0x00, 0x52, 0x08,
	0x6e, 0x65, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x12, 0x46, 0x0a, 0x0b, 0x75, 0x73, 0x65, 0x5f,
	0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x18, 0x04, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x23, 0x2e,
	0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69, 0x6e, 0x76, 0x6f, 0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e,
	0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2e, 0x55, 0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75,
	0x6c, 0x74, 0x48, 0x00, 0x52, 0x0a, 0x75, 0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74,
	0x42, 0x13, 0x0a, 0x11, 0x72, 0x65, 0x70, 0x6c, 0x61, 0x63, 0x65, 0x6d, 0x65, 0x6e, 0x74, 0x5f,
	0x76, 0x61, 0x6c, 0x75, 0x65, 0x4a, 0x04, 0x08, 0x02, 0x10, 0x03, 0x22, 0xb6, 0x01, 0x0a, 0x0b,
	0x41, 0x6c, 0x6c, 0x6f, 0x77, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x12, 0x25, 0x0a, 0x0e, 0x61,
	0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x73, 0x18, 0x01, 0x20,
	0x03, 0x28, 0x09, 0x52, 0x0d, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64, 0x56, 0x61, 0x6c, 0x75,
	0x65, 0x73, 0x12, 0x1d, 0x0a, 0x09, 0x6e, 0x65, 0x77, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x18,
	0x03, 0x20, 0x01, 0x28, 0x09, 0x48, 0x00, 0x52, 0x08, 0x6e, 0x65, 0x77, 0x56, 0x61, 0x6c, 0x75,
	0x65, 0x12, 0x46, 0x0a, 0x0b, 0x75, 0x73, 0x65, 0x5f, 0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74,
	0x18, 0x04, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x23, 0x2e, 0x62, 0x6c, 0x61, 0x7a, 0x65, 0x2e, 0x69,
	0x6e, 0x76, 0x6f, 0x63, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79,
	0x2e, 0x55, 0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x48, 0x00, 0x52, 0x0a, 0x75,
	0x73, 0x65, 0x44, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x42, 0x13, 0x0a, 0x11, 0x72, 0x65, 0x70,
	0x6c, 0x61, 0x63, 0x65, 0x6d, 0x65, 0x6e, 0x74, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x4a, 0x04,
	0x08, 0x02, 0x10, 0x03,
}

var (
	file_bazel_invocation_policy_invocation_policy_proto_rawDescOnce sync.Once
	file_bazel_invocation_policy_invocation_policy_proto_rawDescData = file_bazel_invocation_policy_invocation_policy_proto_rawDesc
)

func file_bazel_invocation_policy_invocation_policy_proto_rawDescGZIP() []byte {
	file_bazel_invocation_policy_invocation_policy_proto_rawDescOnce.Do(func() {
		file_bazel_invocation_policy_invocation_policy_proto_rawDescData = protoimpl.X.CompressGZIP(file_bazel_invocation_policy_invocation_policy_proto_rawDescData)
	})
	return file_bazel_invocation_policy_invocation_policy_proto_rawDescData
}

var file_bazel_invocation_policy_invocation_policy_proto_msgTypes = make([]protoimpl.MessageInfo, 6)
var file_bazel_invocation_policy_invocation_policy_proto_goTypes = []interface{}{
	(*InvocationPolicy)(nil), // 0: blaze.invocation_policy.InvocationPolicy
	(*FlagPolicy)(nil),       // 1: blaze.invocation_policy.FlagPolicy
	(*SetValue)(nil),         // 2: blaze.invocation_policy.SetValue
	(*UseDefault)(nil),       // 3: blaze.invocation_policy.UseDefault
	(*DisallowValues)(nil),   // 4: blaze.invocation_policy.DisallowValues
	(*AllowValues)(nil),      // 5: blaze.invocation_policy.AllowValues
}
var file_bazel_invocation_policy_invocation_policy_proto_depIdxs = []int32{
	1, // 0: blaze.invocation_policy.InvocationPolicy.flag_policies:type_name -> blaze.invocation_policy.FlagPolicy
	2, // 1: blaze.invocation_policy.FlagPolicy.set_value:type_name -> blaze.invocation_policy.SetValue
	3, // 2: blaze.invocation_policy.FlagPolicy.use_default:type_name -> blaze.invocation_policy.UseDefault
	4, // 3: blaze.invocation_policy.FlagPolicy.disallow_values:type_name -> blaze.invocation_policy.DisallowValues
	5, // 4: blaze.invocation_policy.FlagPolicy.allow_values:type_name -> blaze.invocation_policy.AllowValues
	3, // 5: blaze.invocation_policy.DisallowValues.use_default:type_name -> blaze.invocation_policy.UseDefault
	3, // 6: blaze.invocation_policy.AllowValues.use_default:type_name -> blaze.invocation_policy.UseDefault
	7, // [7:7] is the sub-list for method output_type
	7, // [7:7] is the sub-list for method input_type
	7, // [7:7] is the sub-list for extension type_name
	7, // [7:7] is the sub-list for extension extendee
	0, // [0:7] is the sub-list for field type_name
}

func init() { file_bazel_invocation_policy_invocation_policy_proto_init() }
func file_bazel_invocation_policy_invocation_policy_proto_init() {
	if File_bazel_invocation_policy_invocation_policy_proto != nil {
		return
	}
	if !protoimpl.UnsafeEnabled {
		file_bazel_invocation_policy_invocation_policy_proto_msgTypes[0].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*InvocationPolicy); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_bazel_invocation_policy_invocation_policy_proto_msgTypes[1].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*FlagPolicy); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_bazel_invocation_policy_invocation_policy_proto_msgTypes[2].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*SetValue); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_bazel_invocation_policy_invocation_policy_proto_msgTypes[3].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*UseDefault); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_bazel_invocation_policy_invocation_policy_proto_msgTypes[4].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*DisallowValues); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_bazel_invocation_policy_invocation_policy_proto_msgTypes[5].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*AllowValues); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
	}
	file_bazel_invocation_policy_invocation_policy_proto_msgTypes[1].OneofWrappers = []interface{}{
		(*FlagPolicy_SetValue)(nil),
		(*FlagPolicy_UseDefault)(nil),
		(*FlagPolicy_DisallowValues)(nil),
		(*FlagPolicy_AllowValues)(nil),
	}
	file_bazel_invocation_policy_invocation_policy_proto_msgTypes[4].OneofWrappers = []interface{}{
		(*DisallowValues_NewValue)(nil),
		(*DisallowValues_UseDefault)(nil),
	}
	file_bazel_invocation_policy_invocation_policy_proto_msgTypes[5].OneofWrappers = []interface{}{
		(*AllowValues_NewValue)(nil),
		(*AllowValues_UseDefault)(nil),
	}
	type x struct{}
	out := protoimpl.TypeBuilder{
		File: protoimpl.DescBuilder{
			GoPackagePath: reflect.TypeOf(x{}).PkgPath(),
			RawDescriptor: file_bazel_invocation_policy_invocation_policy_proto_rawDesc,
			NumEnums:      0,
			NumMessages:   6,
			NumExtensions: 0,
			NumServices:   0,
		},
		GoTypes:           file_bazel_invocation_policy_invocation_policy_proto_goTypes,
		DependencyIndexes: file_bazel_invocation_policy_invocation_policy_proto_depIdxs,
		MessageInfos:      file_bazel_invocation_policy_invocation_policy_proto_msgTypes,
	}.Build()
	File_bazel_invocation_policy_invocation_policy_proto = out.File
	file_bazel_invocation_policy_invocation_policy_proto_rawDesc = nil
	file_bazel_invocation_policy_invocation_policy_proto_goTypes = nil
	file_bazel_invocation_policy_invocation_policy_proto_depIdxs = nil
}
