# Copyright 2020 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""A rule for generating a `FileDescriptorSet` with all transitive dependencies.

This module contains the definition of `proto_descriptor_set`, a rule that
collects all `FileDescriptorSet`s from its transitive dependencies and generates
a single `FileDescriptorSet` containing all the `FileDescriptorProto` from them.

Based on https://github.com/bazelbuild/rules_proto/blob/main/proto/private/rules/proto_descriptor_set.bzl
but uses bash instead of c binary. The c binary used by that rule lacked headers needed to compile
with our c toolchain setup.
"""

load("@rules_proto//proto:defs.bzl", "ProtoInfo")

def _proto_descriptor_set_impl(ctx):
    output = ctx.actions.declare_file("{}.bin".format(ctx.attr.name))

    descriptor_sets = depset(
        transitive = [dep[ProtoInfo].transitive_descriptor_sets for dep in ctx.attr.deps],
    )

    descriptor_files = descriptor_sets.to_list()

    # Create the command to concatenate files
    ctx.actions.run_shell(
        mnemonic = "ConcatFileDescriptorSet",
        inputs = descriptor_files,
        outputs = [output],
        command = "cat {} > {}".format(
            " ".join([f.path for f in descriptor_files]),
            output.path,
        ),
    )

    return [
        DefaultInfo(
            files = depset([output]),
            runfiles = ctx.runfiles(files = [output]),
        ),
    ]

descriptor_set = rule(
    implementation = _proto_descriptor_set_impl,
    attrs = {
        "deps": attr.label_list(
            mandatory = True,
            providers = [ProtoInfo],
            doc = """
Sequence of `ProtoInfo`s to collect `FileDescriptorSet`s from.
""".strip(),
        ),
    },
    doc = """
Collects all `FileDescriptorSet`s from `deps` and combines them into a single
`FileDescriptorSet` containing all the `FileDescriptorProto`.
""".strip(),
)
