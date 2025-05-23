// Code generated by protoc-gen-go. DO NOT EDIT.
// versions:
// 	protoc-gen-go v1.33.0
// 	protoc        v5.27.3
// source: bazel/command_line/command_line.proto

package command_line

import (
	options "github.com/aspect-build/aspect-cli/bazel/options"
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

type CommandLine struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	CommandLineLabel string                `protobuf:"bytes,1,opt,name=command_line_label,json=commandLineLabel,proto3" json:"command_line_label,omitempty"`
	Sections         []*CommandLineSection `protobuf:"bytes,2,rep,name=sections,proto3" json:"sections,omitempty"`
}

func (x *CommandLine) Reset() {
	*x = CommandLine{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_command_line_command_line_proto_msgTypes[0]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *CommandLine) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*CommandLine) ProtoMessage() {}

func (x *CommandLine) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_command_line_command_line_proto_msgTypes[0]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use CommandLine.ProtoReflect.Descriptor instead.
func (*CommandLine) Descriptor() ([]byte, []int) {
	return file_bazel_command_line_command_line_proto_rawDescGZIP(), []int{0}
}

func (x *CommandLine) GetCommandLineLabel() string {
	if x != nil {
		return x.CommandLineLabel
	}
	return ""
}

func (x *CommandLine) GetSections() []*CommandLineSection {
	if x != nil {
		return x.Sections
	}
	return nil
}

type CommandLineSection struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	SectionLabel string `protobuf:"bytes,1,opt,name=section_label,json=sectionLabel,proto3" json:"section_label,omitempty"`
	// Types that are assignable to SectionType:
	//
	//	*CommandLineSection_ChunkList
	//	*CommandLineSection_OptionList
	SectionType isCommandLineSection_SectionType `protobuf_oneof:"section_type"`
}

func (x *CommandLineSection) Reset() {
	*x = CommandLineSection{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_command_line_command_line_proto_msgTypes[1]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *CommandLineSection) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*CommandLineSection) ProtoMessage() {}

func (x *CommandLineSection) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_command_line_command_line_proto_msgTypes[1]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use CommandLineSection.ProtoReflect.Descriptor instead.
func (*CommandLineSection) Descriptor() ([]byte, []int) {
	return file_bazel_command_line_command_line_proto_rawDescGZIP(), []int{1}
}

func (x *CommandLineSection) GetSectionLabel() string {
	if x != nil {
		return x.SectionLabel
	}
	return ""
}

func (m *CommandLineSection) GetSectionType() isCommandLineSection_SectionType {
	if m != nil {
		return m.SectionType
	}
	return nil
}

func (x *CommandLineSection) GetChunkList() *ChunkList {
	if x, ok := x.GetSectionType().(*CommandLineSection_ChunkList); ok {
		return x.ChunkList
	}
	return nil
}

func (x *CommandLineSection) GetOptionList() *OptionList {
	if x, ok := x.GetSectionType().(*CommandLineSection_OptionList); ok {
		return x.OptionList
	}
	return nil
}

type isCommandLineSection_SectionType interface {
	isCommandLineSection_SectionType()
}

type CommandLineSection_ChunkList struct {
	ChunkList *ChunkList `protobuf:"bytes,2,opt,name=chunk_list,json=chunkList,proto3,oneof"`
}

type CommandLineSection_OptionList struct {
	OptionList *OptionList `protobuf:"bytes,3,opt,name=option_list,json=optionList,proto3,oneof"`
}

func (*CommandLineSection_ChunkList) isCommandLineSection_SectionType() {}

func (*CommandLineSection_OptionList) isCommandLineSection_SectionType() {}

type ChunkList struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Chunk []string `protobuf:"bytes,1,rep,name=chunk,proto3" json:"chunk,omitempty"`
}

func (x *ChunkList) Reset() {
	*x = ChunkList{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_command_line_command_line_proto_msgTypes[2]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *ChunkList) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*ChunkList) ProtoMessage() {}

func (x *ChunkList) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_command_line_command_line_proto_msgTypes[2]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use ChunkList.ProtoReflect.Descriptor instead.
func (*ChunkList) Descriptor() ([]byte, []int) {
	return file_bazel_command_line_command_line_proto_rawDescGZIP(), []int{2}
}

func (x *ChunkList) GetChunk() []string {
	if x != nil {
		return x.Chunk
	}
	return nil
}

type OptionList struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Option []*Option `protobuf:"bytes,1,rep,name=option,proto3" json:"option,omitempty"`
}

func (x *OptionList) Reset() {
	*x = OptionList{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_command_line_command_line_proto_msgTypes[3]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *OptionList) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*OptionList) ProtoMessage() {}

func (x *OptionList) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_command_line_command_line_proto_msgTypes[3]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use OptionList.ProtoReflect.Descriptor instead.
func (*OptionList) Descriptor() ([]byte, []int) {
	return file_bazel_command_line_command_line_proto_rawDescGZIP(), []int{3}
}

func (x *OptionList) GetOption() []*Option {
	if x != nil {
		return x.Option
	}
	return nil
}

type Option struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	CombinedForm string                      `protobuf:"bytes,1,opt,name=combined_form,json=combinedForm,proto3" json:"combined_form,omitempty"`
	OptionName   string                      `protobuf:"bytes,2,opt,name=option_name,json=optionName,proto3" json:"option_name,omitempty"`
	OptionValue  string                      `protobuf:"bytes,3,opt,name=option_value,json=optionValue,proto3" json:"option_value,omitempty"`
	EffectTags   []options.OptionEffectTag   `protobuf:"varint,4,rep,packed,name=effect_tags,json=effectTags,proto3,enum=options.OptionEffectTag" json:"effect_tags,omitempty"`
	MetadataTags []options.OptionMetadataTag `protobuf:"varint,5,rep,packed,name=metadata_tags,json=metadataTags,proto3,enum=options.OptionMetadataTag" json:"metadata_tags,omitempty"`
}

func (x *Option) Reset() {
	*x = Option{}
	if protoimpl.UnsafeEnabled {
		mi := &file_bazel_command_line_command_line_proto_msgTypes[4]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *Option) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*Option) ProtoMessage() {}

func (x *Option) ProtoReflect() protoreflect.Message {
	mi := &file_bazel_command_line_command_line_proto_msgTypes[4]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use Option.ProtoReflect.Descriptor instead.
func (*Option) Descriptor() ([]byte, []int) {
	return file_bazel_command_line_command_line_proto_rawDescGZIP(), []int{4}
}

func (x *Option) GetCombinedForm() string {
	if x != nil {
		return x.CombinedForm
	}
	return ""
}

func (x *Option) GetOptionName() string {
	if x != nil {
		return x.OptionName
	}
	return ""
}

func (x *Option) GetOptionValue() string {
	if x != nil {
		return x.OptionValue
	}
	return ""
}

func (x *Option) GetEffectTags() []options.OptionEffectTag {
	if x != nil {
		return x.EffectTags
	}
	return nil
}

func (x *Option) GetMetadataTags() []options.OptionMetadataTag {
	if x != nil {
		return x.MetadataTags
	}
	return nil
}

var File_bazel_command_line_command_line_proto protoreflect.FileDescriptor

var file_bazel_command_line_command_line_proto_rawDesc = []byte{
	0x0a, 0x25, 0x62, 0x61, 0x7a, 0x65, 0x6c, 0x2f, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x5f,
	0x6c, 0x69, 0x6e, 0x65, 0x2f, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x5f, 0x6c, 0x69, 0x6e,
	0x65, 0x2e, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x12, 0x0c, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64,
	0x5f, 0x6c, 0x69, 0x6e, 0x65, 0x1a, 0x22, 0x62, 0x61, 0x7a, 0x65, 0x6c, 0x2f, 0x6f, 0x70, 0x74,
	0x69, 0x6f, 0x6e, 0x73, 0x2f, 0x6f, 0x70, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x66, 0x69, 0x6c, 0x74,
	0x65, 0x72, 0x73, 0x2e, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x22, 0x79, 0x0a, 0x0b, 0x43, 0x6f, 0x6d,
	0x6d, 0x61, 0x6e, 0x64, 0x4c, 0x69, 0x6e, 0x65, 0x12, 0x2c, 0x0a, 0x12, 0x63, 0x6f, 0x6d, 0x6d,
	0x61, 0x6e, 0x64, 0x5f, 0x6c, 0x69, 0x6e, 0x65, 0x5f, 0x6c, 0x61, 0x62, 0x65, 0x6c, 0x18, 0x01,
	0x20, 0x01, 0x28, 0x09, 0x52, 0x10, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x4c, 0x69, 0x6e,
	0x65, 0x4c, 0x61, 0x62, 0x65, 0x6c, 0x12, 0x3c, 0x0a, 0x08, 0x73, 0x65, 0x63, 0x74, 0x69, 0x6f,
	0x6e, 0x73, 0x18, 0x02, 0x20, 0x03, 0x28, 0x0b, 0x32, 0x20, 0x2e, 0x63, 0x6f, 0x6d, 0x6d, 0x61,
	0x6e, 0x64, 0x5f, 0x6c, 0x69, 0x6e, 0x65, 0x2e, 0x43, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x4c,
	0x69, 0x6e, 0x65, 0x53, 0x65, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x52, 0x08, 0x73, 0x65, 0x63, 0x74,
	0x69, 0x6f, 0x6e, 0x73, 0x22, 0xc0, 0x01, 0x0a, 0x12, 0x43, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64,
	0x4c, 0x69, 0x6e, 0x65, 0x53, 0x65, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x12, 0x23, 0x0a, 0x0d, 0x73,
	0x65, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x6c, 0x61, 0x62, 0x65, 0x6c, 0x18, 0x01, 0x20, 0x01,
	0x28, 0x09, 0x52, 0x0c, 0x73, 0x65, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x4c, 0x61, 0x62, 0x65, 0x6c,
	0x12, 0x38, 0x0a, 0x0a, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x5f, 0x6c, 0x69, 0x73, 0x74, 0x18, 0x02,
	0x20, 0x01, 0x28, 0x0b, 0x32, 0x17, 0x2e, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x5f, 0x6c,
	0x69, 0x6e, 0x65, 0x2e, 0x43, 0x68, 0x75, 0x6e, 0x6b, 0x4c, 0x69, 0x73, 0x74, 0x48, 0x00, 0x52,
	0x09, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x4c, 0x69, 0x73, 0x74, 0x12, 0x3b, 0x0a, 0x0b, 0x6f, 0x70,
	0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x6c, 0x69, 0x73, 0x74, 0x18, 0x03, 0x20, 0x01, 0x28, 0x0b, 0x32,
	0x18, 0x2e, 0x63, 0x6f, 0x6d, 0x6d, 0x61, 0x6e, 0x64, 0x5f, 0x6c, 0x69, 0x6e, 0x65, 0x2e, 0x4f,
	0x70, 0x74, 0x69, 0x6f, 0x6e, 0x4c, 0x69, 0x73, 0x74, 0x48, 0x00, 0x52, 0x0a, 0x6f, 0x70, 0x74,
	0x69, 0x6f, 0x6e, 0x4c, 0x69, 0x73, 0x74, 0x42, 0x0e, 0x0a, 0x0c, 0x73, 0x65, 0x63, 0x74, 0x69,
	0x6f, 0x6e, 0x5f, 0x74, 0x79, 0x70, 0x65, 0x22, 0x21, 0x0a, 0x09, 0x43, 0x68, 0x75, 0x6e, 0x6b,
	0x4c, 0x69, 0x73, 0x74, 0x12, 0x14, 0x0a, 0x05, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x18, 0x01, 0x20,
	0x03, 0x28, 0x09, 0x52, 0x05, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x22, 0x3a, 0x0a, 0x0a, 0x4f, 0x70,
	0x74, 0x69, 0x6f, 0x6e, 0x4c, 0x69, 0x73, 0x74, 0x12, 0x2c, 0x0a, 0x06, 0x6f, 0x70, 0x74, 0x69,
	0x6f, 0x6e, 0x18, 0x01, 0x20, 0x03, 0x28, 0x0b, 0x32, 0x14, 0x2e, 0x63, 0x6f, 0x6d, 0x6d, 0x61,
	0x6e, 0x64, 0x5f, 0x6c, 0x69, 0x6e, 0x65, 0x2e, 0x4f, 0x70, 0x74, 0x69, 0x6f, 0x6e, 0x52, 0x06,
	0x6f, 0x70, 0x74, 0x69, 0x6f, 0x6e, 0x22, 0xed, 0x01, 0x0a, 0x06, 0x4f, 0x70, 0x74, 0x69, 0x6f,
	0x6e, 0x12, 0x23, 0x0a, 0x0d, 0x63, 0x6f, 0x6d, 0x62, 0x69, 0x6e, 0x65, 0x64, 0x5f, 0x66, 0x6f,
	0x72, 0x6d, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x0c, 0x63, 0x6f, 0x6d, 0x62, 0x69, 0x6e,
	0x65, 0x64, 0x46, 0x6f, 0x72, 0x6d, 0x12, 0x1f, 0x0a, 0x0b, 0x6f, 0x70, 0x74, 0x69, 0x6f, 0x6e,
	0x5f, 0x6e, 0x61, 0x6d, 0x65, 0x18, 0x02, 0x20, 0x01, 0x28, 0x09, 0x52, 0x0a, 0x6f, 0x70, 0x74,
	0x69, 0x6f, 0x6e, 0x4e, 0x61, 0x6d, 0x65, 0x12, 0x21, 0x0a, 0x0c, 0x6f, 0x70, 0x74, 0x69, 0x6f,
	0x6e, 0x5f, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x18, 0x03, 0x20, 0x01, 0x28, 0x09, 0x52, 0x0b, 0x6f,
	0x70, 0x74, 0x69, 0x6f, 0x6e, 0x56, 0x61, 0x6c, 0x75, 0x65, 0x12, 0x39, 0x0a, 0x0b, 0x65, 0x66,
	0x66, 0x65, 0x63, 0x74, 0x5f, 0x74, 0x61, 0x67, 0x73, 0x18, 0x04, 0x20, 0x03, 0x28, 0x0e, 0x32,
	0x18, 0x2e, 0x6f, 0x70, 0x74, 0x69, 0x6f, 0x6e, 0x73, 0x2e, 0x4f, 0x70, 0x74, 0x69, 0x6f, 0x6e,
	0x45, 0x66, 0x66, 0x65, 0x63, 0x74, 0x54, 0x61, 0x67, 0x52, 0x0a, 0x65, 0x66, 0x66, 0x65, 0x63,
	0x74, 0x54, 0x61, 0x67, 0x73, 0x12, 0x3f, 0x0a, 0x0d, 0x6d, 0x65, 0x74, 0x61, 0x64, 0x61, 0x74,
	0x61, 0x5f, 0x74, 0x61, 0x67, 0x73, 0x18, 0x05, 0x20, 0x03, 0x28, 0x0e, 0x32, 0x1a, 0x2e, 0x6f,
	0x70, 0x74, 0x69, 0x6f, 0x6e, 0x73, 0x2e, 0x4f, 0x70, 0x74, 0x69, 0x6f, 0x6e, 0x4d, 0x65, 0x74,
	0x61, 0x64, 0x61, 0x74, 0x61, 0x54, 0x61, 0x67, 0x52, 0x0c, 0x6d, 0x65, 0x74, 0x61, 0x64, 0x61,
	0x74, 0x61, 0x54, 0x61, 0x67, 0x73, 0x62, 0x06, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x33,
}

var (
	file_bazel_command_line_command_line_proto_rawDescOnce sync.Once
	file_bazel_command_line_command_line_proto_rawDescData = file_bazel_command_line_command_line_proto_rawDesc
)

func file_bazel_command_line_command_line_proto_rawDescGZIP() []byte {
	file_bazel_command_line_command_line_proto_rawDescOnce.Do(func() {
		file_bazel_command_line_command_line_proto_rawDescData = protoimpl.X.CompressGZIP(file_bazel_command_line_command_line_proto_rawDescData)
	})
	return file_bazel_command_line_command_line_proto_rawDescData
}

var file_bazel_command_line_command_line_proto_msgTypes = make([]protoimpl.MessageInfo, 5)
var file_bazel_command_line_command_line_proto_goTypes = []interface{}{
	(*CommandLine)(nil),            // 0: command_line.CommandLine
	(*CommandLineSection)(nil),     // 1: command_line.CommandLineSection
	(*ChunkList)(nil),              // 2: command_line.ChunkList
	(*OptionList)(nil),             // 3: command_line.OptionList
	(*Option)(nil),                 // 4: command_line.Option
	(options.OptionEffectTag)(0),   // 5: options.OptionEffectTag
	(options.OptionMetadataTag)(0), // 6: options.OptionMetadataTag
}
var file_bazel_command_line_command_line_proto_depIdxs = []int32{
	1, // 0: command_line.CommandLine.sections:type_name -> command_line.CommandLineSection
	2, // 1: command_line.CommandLineSection.chunk_list:type_name -> command_line.ChunkList
	3, // 2: command_line.CommandLineSection.option_list:type_name -> command_line.OptionList
	4, // 3: command_line.OptionList.option:type_name -> command_line.Option
	5, // 4: command_line.Option.effect_tags:type_name -> options.OptionEffectTag
	6, // 5: command_line.Option.metadata_tags:type_name -> options.OptionMetadataTag
	6, // [6:6] is the sub-list for method output_type
	6, // [6:6] is the sub-list for method input_type
	6, // [6:6] is the sub-list for extension type_name
	6, // [6:6] is the sub-list for extension extendee
	0, // [0:6] is the sub-list for field type_name
}

func init() { file_bazel_command_line_command_line_proto_init() }
func file_bazel_command_line_command_line_proto_init() {
	if File_bazel_command_line_command_line_proto != nil {
		return
	}
	if !protoimpl.UnsafeEnabled {
		file_bazel_command_line_command_line_proto_msgTypes[0].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*CommandLine); i {
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
		file_bazel_command_line_command_line_proto_msgTypes[1].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*CommandLineSection); i {
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
		file_bazel_command_line_command_line_proto_msgTypes[2].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*ChunkList); i {
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
		file_bazel_command_line_command_line_proto_msgTypes[3].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*OptionList); i {
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
		file_bazel_command_line_command_line_proto_msgTypes[4].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*Option); i {
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
	file_bazel_command_line_command_line_proto_msgTypes[1].OneofWrappers = []interface{}{
		(*CommandLineSection_ChunkList)(nil),
		(*CommandLineSection_OptionList)(nil),
	}
	type x struct{}
	out := protoimpl.TypeBuilder{
		File: protoimpl.DescBuilder{
			GoPackagePath: reflect.TypeOf(x{}).PkgPath(),
			RawDescriptor: file_bazel_command_line_command_line_proto_rawDesc,
			NumEnums:      0,
			NumMessages:   5,
			NumExtensions: 0,
			NumServices:   0,
		},
		GoTypes:           file_bazel_command_line_command_line_proto_goTypes,
		DependencyIndexes: file_bazel_command_line_command_line_proto_depIdxs,
		MessageInfos:      file_bazel_command_line_command_line_proto_msgTypes,
	}.Build()
	File_bazel_command_line_command_line_proto = out.File
	file_bazel_command_line_command_line_proto_rawDesc = nil
	file_bazel_command_line_command_line_proto_goTypes = nil
	file_bazel_command_line_command_line_proto_depIdxs = nil
}
