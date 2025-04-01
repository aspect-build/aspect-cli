// Code generated by protoc-gen-go. DO NOT EDIT.
// versions:
// 	protoc-gen-go v1.33.0
// 	protoc        v5.27.3
// source: bazel/flags/flags.proto

package flags

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

type FlagInfo struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Name                  *string  `protobuf:"bytes,1,req,name=name" json:"name,omitempty"`
	HasNegativeFlag       *bool    `protobuf:"varint,2,opt,name=has_negative_flag,json=hasNegativeFlag,def=0" json:"has_negative_flag,omitempty"`
	Documentation         *string  `protobuf:"bytes,3,opt,name=documentation" json:"documentation,omitempty"`
	Commands              []string `protobuf:"bytes,4,rep,name=commands" json:"commands,omitempty"`
	Abbreviation          *string  `protobuf:"bytes,5,opt,name=abbreviation" json:"abbreviation,omitempty"`
	AllowsMultiple        *bool    `protobuf:"varint,6,opt,name=allows_multiple,json=allowsMultiple,def=0" json:"allows_multiple,omitempty"`
	EffectTags            []string `protobuf:"bytes,7,rep,name=effect_tags,json=effectTags" json:"effect_tags,omitempty"`
	MetadataTags          []string `protobuf:"bytes,8,rep,name=metadata_tags,json=metadataTags" json:"metadata_tags,omitempty"`
	DocumentationCategory *string  `protobuf:"bytes,9,opt,name=documentation_category,json=documentationCategory" json:"documentation_category,omitempty"`
	RequiresValue         *bool    `protobuf:"varint,10,opt,name=requires_value,json=requiresValue" json:"requires_value,omitempty"`
}

// Default values for FlagInfo fields.
const (
	Default_FlagInfo_HasNegativeFlag = bool(false)
	Default_FlagInfo_AllowsMultiple  = bool(false)
)

func (x *FlagInfo) Reset() {
	*x = FlagInfo{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_flags_flags_proto_msgTypes[0]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *FlagInfo) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*FlagInfo) ProtoMessage() {}

func (x *FlagInfo) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_flags_flags_proto_msgTypes[0]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use FlagInfo.ProtoReflect.Descriptor instead.
func (*FlagInfo) Descriptor() ([]byte, []int) {
	return file_bazel_flags_flags_proto_rawDescGZIP(), []int{0}
}

func (x *FlagInfo) GetName() string {
	if x != nil && x.Name != nil {
		return *x.Name
	}
	return ""
}

func (x *FlagInfo) GetHasNegativeFlag() bool {
	if x != nil && x.HasNegativeFlag != nil {
		return *x.HasNegativeFlag
	}
	return Default_FlagInfo_HasNegativeFlag
}

func (x *FlagInfo) GetDocumentation() string {
	if x != nil && x.Documentation != nil {
		return *x.Documentation
	}
	return ""
}

func (x *FlagInfo) GetCommands() []string {
	if x != nil {
		return x.Commands
	}
	return nil
}

func (x *FlagInfo) GetAbbreviation() string {
	if x != nil && x.Abbreviation != nil {
		return *x.Abbreviation
	}
	return ""
}

func (x *FlagInfo) GetAllowsMultiple() bool {
	if x != nil && x.AllowsMultiple != nil {
		return *x.AllowsMultiple
	}
	return Default_FlagInfo_AllowsMultiple
}

func (x *FlagInfo) GetEffectTags() []string {
	if x != nil {
		return x.EffectTags
	}
	return nil
}

func (x *FlagInfo) GetMetadataTags() []string {
	if x != nil {
		return x.MetadataTags
	}
	return nil
}

func (x *FlagInfo) GetDocumentationCategory() string {
	if x != nil && x.DocumentationCategory != nil {
		return *x.DocumentationCategory
	}
	return ""
}

func (x *FlagInfo) GetRequiresValue() bool {
	if x != nil && x.RequiresValue != nil {
		return *x.RequiresValue
	}
	return false
}

type FlagCollection struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	FlagInfos []*FlagInfo `protobuf:"bytes,1,rep,name=flag_infos,json=flagInfos" json:"flag_infos,omitempty"`
}

func (x *FlagCollection) Reset() {
	*x = FlagCollection{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_flags_flags_proto_msgTypes[1]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *FlagCollection) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*FlagCollection) ProtoMessage() {}

func (x *FlagCollection) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_flags_flags_proto_msgTypes[1]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use FlagCollection.ProtoReflect.Descriptor instead.
func (*FlagCollection) Descriptor() ([]byte, []int) {
	return file_bazel_flags_flags_proto_rawDescGZIP(), []int{1}
}

func (x *FlagCollection) GetFlagInfos() []*FlagInfo {
	if x != nil {
		return x.FlagInfos
	}
	return nil
}

var File_bazel_flags_flags_proto protoreflect.FileDescriptor

var file_bazel_flags_flags_proto_rawDesc = []byte{
	0x0a, 0x17, 0x62, 0x61, 0x7a, 0x65, 0x6c, 0x2f, 0x66, 0x6c, 0x61, 0x67, 0x73, 0x2f, 0x66, 0x6c,
	0x61, 0x67, 0x73, 0x2e, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x12, 0x05, 0x62, 0x61, 0x7a, 0x65, 0x6c,
	0x22, 0x8b, 0x03, 0x0a, 0x08, 0x46, 0x6c, 0x61, 0x67, 0x49, 0x6e, 0x66, 0x6f, 0x12, 0x12, 0x0a,
	0x04, 0x6e, 0x61, 0x6d, 0x65, 0x18, 0x01, 0x20, 0x02, 0x28, 0x09, 0x52, 0x04, 0x6e, 0x61, 0x6d,
	0x65, 0x12, 0x31, 0x0a, 0x11, 0x68, 0x61, 0x73, 0x5f, 0x6e, 0x65, 0x67, 0x61, 0x74, 0x69, 0x76,
	0x65, 0x5f, 0x66, 0x6c, 0x61, 0x67, 0x18, 0x02, 0x20, 0x01, 0x28, 0x08, 0x3a, 0x05, 0x66, 0x61,
	0x6c, 0x73, 0x65, 0x52, 0x0f, 0x68, 0x61, 0x73, 0x4e, 0x65, 0x67, 0x61, 0x74, 0x69, 0x76, 0x65,
	0x46, 0x6c, 0x61, 0x67, 0x12, 0x24, 0x0a, 0x0d, 0x64, 0x6f, 0x63, 0x75, 0x6d, 0x65, 0x6e, 0x74,
	0x61, 0x74, 0x69, 0x6f, 0x6e, 0x18, 0x03, 0x20, 0x01, 0x28, 0x09, 0x52, 0x0d, 0x64, 0x6f, 0x63,
	0x75, 0x6d, 0x65, 0x6e, 0x74, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x12, 0x1a, 0x0a, 0x08, 0x63, 0x6f,
	0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x73, 0x18, 0x04, 0x20, 0x03, 0x28, 0x09, 0x52, 0x08, 0x63, 0x6f,
	0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x73, 0x12, 0x22, 0x0a, 0x0c, 0x61, 0x62, 0x62, 0x72, 0x65, 0x76,
	0x69, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x18, 0x05, 0x20, 0x01, 0x28, 0x09, 0x52, 0x0c, 0x61, 0x62,
	0x62, 0x72, 0x65, 0x76, 0x69, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x12, 0x2e, 0x0a, 0x0f, 0x61, 0x6c,
	0x6c, 0x6f, 0x77, 0x73, 0x5f, 0x6d, 0x75, 0x6c, 0x74, 0x69, 0x70, 0x6c, 0x65, 0x18, 0x06, 0x20,
	0x01, 0x28, 0x08, 0x3a, 0x05, 0x66, 0x61, 0x6c, 0x73, 0x65, 0x52, 0x0e, 0x61, 0x6c, 0x6c, 0x6f,
	0x77, 0x73, 0x4d, 0x75, 0x6c, 0x74, 0x69, 0x70, 0x6c, 0x65, 0x12, 0x1f, 0x0a, 0x0b, 0x65, 0x66,
	0x66, 0x65, 0x63, 0x74, 0x5f, 0x74, 0x61, 0x67, 0x73, 0x18, 0x07, 0x20, 0x03, 0x28, 0x09, 0x52,
	0x0a, 0x65, 0x66, 0x66, 0x65, 0x63, 0x74, 0x54, 0x61, 0x67, 0x73, 0x12, 0x23, 0x0a, 0x0d, 0x6d,
	0x65, 0x74, 0x61, 0x64, 0x61, 0x74, 0x61, 0x5f, 0x74, 0x61, 0x67, 0x73, 0x18, 0x08, 0x20, 0x03,
	0x28, 0x09, 0x52, 0x0c, 0x6d, 0x65, 0x74, 0x61, 0x64, 0x61, 0x74, 0x61, 0x54, 0x61, 0x67, 0x73,
	0x12, 0x35, 0x0a, 0x16, 0x64, 0x6f, 0x63, 0x75, 0x6d, 0x65, 0x6e, 0x74, 0x61, 0x74, 0x69, 0x6f,
	0x6e, 0x5f, 0x63, 0x61, 0x74, 0x65, 0x67, 0x6f, 0x72, 0x79, 0x18, 0x09, 0x20, 0x01, 0x28, 0x09,
	0x52, 0x15, 0x64, 0x6f, 0x63, 0x75, 0x6d, 0x65, 0x6e, 0x74, 0x61, 0x74, 0x69, 0x6f, 0x6e, 0x43,
	0x61, 0x74, 0x65, 0x67, 0x6f, 0x72, 0x79, 0x12, 0x25, 0x0a, 0x0e, 0x72, 0x65, 0x71, 0x75, 0x69,
	0x72, 0x65, 0x73, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x18, 0x0a, 0x20, 0x01, 0x28, 0x08, 0x52,
	0x0d, 0x72, 0x65, 0x71, 0x75, 0x69, 0x72, 0x65, 0x73, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x22, 0x40,
	0x0a, 0x0e, 0x46, 0x6c, 0x61, 0x67, 0x43, 0x6f, 0x6c, 0x6c, 0x65, 0x63, 0x74, 0x69, 0x6f, 0x6e,
	0x12, 0x2e, 0x0a, 0x0a, 0x66, 0x6c, 0x61, 0x67, 0x5f, 0x69, 0x6e, 0x66, 0x6f, 0x73, 0x18, 0x01,
	0x20, 0x03, 0x28, 0x0b, 0x32, 0x0f, 0x2e, 0x62, 0x61, 0x7a, 0x65, 0x6c, 0x2e, 0x46, 0x6c, 0x61,
	0x67, 0x49, 0x6e, 0x66, 0x6f, 0x52, 0x09, 0x66, 0x6c, 0x61, 0x67, 0x49, 0x6e, 0x66, 0x6f, 0x73,
	0x42, 0x47, 0x0a, 0x34, 0x63, 0x6f, 0x6d, 0x2e, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x2e, 0x64,
	0x65, 0x76, 0x74, 0x6f, 0x6f, 0x6c, 0x73, 0x2e, 0x62, 0x75, 0x69, 0x6c, 0x64, 0x2e, 0x6c, 0x69,
	0x62, 0x2e, 0x72, 0x75, 0x6e, 0x74, 0x69, 0x6d, 0x65, 0x2e, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e,
	0x64, 0x73, 0x2e, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x42, 0x0f, 0x42, 0x61, 0x7a, 0x65, 0x6c, 0x46,
	0x6c, 0x61, 0x67, 0x73, 0x50, 0x72, 0x6f, 0x74, 0x6f,
}

var (
	file_bazel_flags_flags_proto_rawDescOnce sync.Once
	file_bazel_flags_flags_proto_rawDescData = file_bazel_flags_flags_proto_rawDesc
)

func file_bazel_flags_flags_proto_rawDescGZIP() []byte {
	file_bazel_flags_flags_proto_rawDescOnce.Do(func() {
		file_bazel_flags_flags_proto_rawDescData = protoimpl.X.CompressGZIP(file_bazel_flags_flags_proto_rawDescData)
	})
	return file_bazel_flags_flags_proto_rawDescData
}

var file_bazel_flags_flags_proto_msgTypes = make([]protoimpl.MessageInfo, 2)
var file_bazel_flags_flags_proto_goTypes = []interface{}{
	(*FlagInfo)(nil),       // 0: bazel.FlagInfo
	(*FlagCollection)(nil), // 1: bazel.FlagCollection
}
var file_bazel_flags_flags_proto_depIdxs = []int32{
	0, // 0: bazel.FlagCollection.flag_infos:type_name -> bazel.FlagInfo
	1, // [1:1] is the sub-list for method output_type
	1, // [1:1] is the sub-list for method input_type
	1, // [1:1] is the sub-list for extension type_name
	1, // [1:1] is the sub-list for extension extendee
	0, // [0:1] is the sub-list for field type_name
}

func init() { file_bazel_flags_flags_proto_init() }
func file_bazel_flags_flags_proto_init() {
	if File_bazel_flags_flags_proto != nil {
		return
	}
	if !protoimpl.UnsafeEnabled {
		file_bazel_flags_flags_proto_msgTypes[0].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*FlagInfo); i {
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
		file_bazel_flags_flags_proto_msgTypes[1].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*FlagCollection); i {
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
	type x struct{}
	out := protoimpl.TypeBuilder{
		File: protoimpl.DescBuilder{
			GoPackagePath: reflect.TypeOf(x{}).PkgPath(),
			RawDescriptor: file_bazel_flags_flags_proto_rawDesc,
			NumEnums:      0,
			NumMessages:   2,
			NumExtensions: 0,
			NumServices:   0,
		},
		GoTypes:           file_bazel_flags_flags_proto_goTypes,
		DependencyIndexes: file_bazel_flags_flags_proto_depIdxs,
		MessageInfos:      file_bazel_flags_flags_proto_msgTypes,
	}.Build()
	File_bazel_flags_flags_proto = out.File
	file_bazel_flags_flags_proto_rawDesc = nil
	file_bazel_flags_flags_proto_goTypes = nil
	file_bazel_flags_flags_proto_depIdxs = nil
}
