#!/usr/bin/env python3
import base64
import json
import sys

from google.protobuf import descriptor as _descriptor
from google.protobuf.descriptor_pb2 import FileDescriptorSet
from google.protobuf.descriptor_pool import DescriptorPool
from google.protobuf.internal.decoder import _DecodeVarint32
from google.protobuf.message_factory import MessageFactory


def load_message_class(descriptor_path, message_type):
    with open(descriptor_path, "rb") as f:
        fds = FileDescriptorSet.FromString(f.read())

    pool = DescriptorPool()
    for fd in fds.file:
        pool.Add(fd)

    descriptor = pool.FindMessageTypeByName(message_type)
    factory = MessageFactory(pool=pool)
    return factory.GetPrototype(descriptor), pool, factory


def message_to_dict(msg, pool, factory):
    """Convert message to dict, leaving unknown Any types as raw."""
    result = {}
    for field, value in msg.ListFields():
        if field.message_type:
            if field.message_type.full_name == "google.protobuf.Any":
                if field.label == field.LABEL_REPEATED:
                    result[field.name] = [any_to_dict(v, pool, factory) for v in value]
                else:
                    result[field.name] = any_to_dict(value, pool, factory)
            elif field.label == field.LABEL_REPEATED:
                result[field.name] = [message_to_dict(v, pool, factory) for v in value]
            else:
                result[field.name] = message_to_dict(value, pool, factory)
        elif field.type == _descriptor.FieldDescriptor.TYPE_ENUM:
            enum_desc = field.enum_type
            if field.label == field.LABEL_REPEATED:
                result[field.name] = [enum_desc.values_by_number[v].name for v in value]
            else:
                result[field.name] = enum_desc.values_by_number[value].name
        elif field.type == _descriptor.FieldDescriptor.TYPE_BYTES:
            if field.label == field.LABEL_REPEATED:
                result[field.name] = [
                    base64.b64encode(v).decode("ascii") for v in value
                ]
            else:
                result[field.name] = base64.b64encode(value).decode("ascii")
        else:
            result[field.name] = value
    return result


def any_to_dict(any_msg, pool, factory):
    """Convert Any message, falling back to raw if type unknown."""
    type_url = any_msg.type_url
    type_name = type_url.split("/")[-1]

    try:
        msg_descriptor = pool.FindMessageTypeByName(type_name)
        msg_class = factory.GetPrototype(msg_descriptor)
        msg = msg_class()
        msg.ParseFromString(any_msg.value)
        return {"@type": type_url, **message_to_dict(msg, pool, factory)}
    except KeyError:
        # Unknown type - return raw
        return {
            "@type": type_url,
            "value": base64.b64encode(any_msg.value).decode("ascii"),
        }


def decode_log(filepath, descriptor_path, message_type):
    msg_class, pool, factory = load_message_class(descriptor_path, message_type)

    with open(filepath, "rb") as f:
        data = f.read()

    pos = 0
    results = []
    while pos < len(data):
        length, pos = _DecodeVarint32(data, pos)
        msg = msg_class()
        msg.ParseFromString(data[pos : pos + length])
        results.append(message_to_dict(msg, pool, factory))
        pos += length
    return results


if __name__ == "__main__":
    log_file = sys.argv[1] if len(sys.argv) > 1 else "examples/large_bes/test.log"
    descriptor = "../../crates/axl-proto/descriptor.bin"
    msg_type = "remote_logging.LogEntry"

    entries = decode_log(log_file, descriptor, msg_type)
    print(json.dumps(entries, indent=2))
