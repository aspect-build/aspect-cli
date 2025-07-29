package run

import (
	"reflect"
	"strings"
	"testing"
)

func TestParseExecLog(t *testing.T) {
	// A execlog containing 2 entries copied from a real build
	execLog := `
{
  "commandArgs": ["external/aspect_bazel_lib~~toolchains~copy_to_directory_darwin_arm64/copy_to_directory", "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg_config.json", ""],
  "environmentVariables": [],
  "platform": {
    "properties": []
  },
  "inputs": [{
    "path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg_config.json",
    "digest": {
      "hash": "f997652c26c9e3e1c452604cd74734fcad4e8c25c80ada8c26732cbb976d3acc",
      "sizeBytes": "815",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }, {
    "path": "external/aspect_bazel_lib~~toolchains~copy_to_directory_darwin_arm64/copy_to_directory",
    "digest": {
      "hash": "ca79d49085627df2e2037327cc528653965f9a79f5530e3b1334245c2274898f",
      "sizeBytes": "1932178",
      "hashFunctionName": "SHA-256"
    },
    "isTool": true,
    "symlinkTargetPath": ""
  }, {
    "path": "mypkg/index.js",
    "digest": {
      "hash": "ae73d02cc249608e1ade5f29faf00c673fa4079ae527ab2e03ad4c0265b13553",
      "sizeBytes": "153",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }, {
    "path": "mypkg/package.json",
    "digest": {
      "hash": "0de716f58c78b84a6cad44209f15734e0046adf27c2025da67f4d9ecc660cb7b",
      "sizeBytes": "83",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }],
  "listedOutputs": ["bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg"],
  "remotable": true,
  "cacheable": true,
  "timeoutMillis": "0",
  "mnemonic": "CopyToDirectory",
  "actualOutputs": [{
    "path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/index.js",
    "digest": {
      "hash": "ae73d02cc249608e1ade5f29faf00c673fa4079ae527ab2e03ad4c0265b13553",
      "sizeBytes": "153",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }, {
    "path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/package.json",
    "digest": {
      "hash": "0de716f58c78b84a6cad44209f15734e0046adf27c2025da67f4d9ecc660cb7b",
      "sizeBytes": "83",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }],
  "runner": "local",
  "cacheHit": false,
  "status": "",
  "exitCode": 0,
  "remoteCacheable": true,
  "targetLabel": "//mypkg:pkg",
  "metrics": {
    "totalTime": "0.073s",
    "executionWallTime": "0.072s",
    "inputBytes": "0",
    "inputFiles": "4",
    "memoryEstimateBytes": "0",
    "inputBytesLimit": "0",
    "inputFilesLimit": "0",
    "outputBytesLimit": "0",
    "outputFilesLimit": "0",
    "memoryBytesLimit": "0",
    "startTime": "2025-06-12T00:45:44.766Z"
  }
}{
  "commandArgs": ["external/aspect_bazel_lib~~toolchains~copy_directory_darwin_arm64/copy_directory", "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg", "bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg", "--hardlink"],
  "environmentVariables": [],
  "platform": {
    "properties": []
  },
  "inputs": [{
    "path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/index.js",
    "digest": {
      "hash": "ae73d02cc249608e1ade5f29faf00c673fa4079ae527ab2e03ad4c0265b13553",
      "sizeBytes": "153",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }, {
    "path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/package.json",
    "digest": {
      "hash": "0de716f58c78b84a6cad44209f15734e0046adf27c2025da67f4d9ecc660cb7b",
      "sizeBytes": "83",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }, {
    "path": "external/aspect_bazel_lib~~toolchains~copy_directory_darwin_arm64/copy_directory",
    "digest": {
      "hash": "7987b4ed6e0e519256b3b0ea6ef35b5aad6dd10b50fcc70467ae20b902b00d2c",
      "sizeBytes": "1544418",
      "hashFunctionName": "SHA-256"
    },
    "isTool": true,
    "symlinkTargetPath": ""
  }],
  "listedOutputs": ["bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg"],
  "remotable": true,
  "cacheable": true,
  "timeoutMillis": "0",
  "mnemonic": "CopyDirectory",
  "actualOutputs": [{
    "path": "bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg/index.js",
    "digest": {
      "hash": "ae73d02cc249608e1ade5f29faf00c673fa4079ae527ab2e03ad4c0265b13553",
      "sizeBytes": "153",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }, {
    "path": "bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg/package.json",
    "digest": {
      "hash": "0de716f58c78b84a6cad44209f15734e0046adf27c2025da67f4d9ecc660cb7b",
      "sizeBytes": "83",
      "hashFunctionName": "SHA-256"
    },
    "isTool": false,
    "symlinkTargetPath": ""
  }],
  "runner": "local",
  "cacheHit": false,
  "status": "",
  "exitCode": 0,
  "remoteCacheable": true,
  "targetLabel": "//:.aspect_rules_js/node_modules/@mycorp+mypkg@0.0.0",
  "metrics": {
    "totalTime": "0.042s",
    "executionWallTime": "0.041s",
    "inputBytes": "0",
    "inputFiles": "2",
    "memoryEstimateBytes": "0",
    "inputBytesLimit": "0",
    "inputFilesLimit": "0",
    "outputBytesLimit": "0",
    "outputFilesLimit": "0",
    "memoryBytesLimit": "0",
    "startTime": "2025-06-12T00:45:44.842Z"
  }
}`

	expectedInputs := []string{
		"bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg",
		"bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/index.js",
		"bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/package.json",
		"bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg",
		"bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg/index.js",
		"bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg/package.json",
	}

	r, err := parseExecLogInputs(strings.NewReader(execLog))
	if err != nil {
		t.Errorf("Failed to parse exec log: %v", err)
	}

	if !reflect.DeepEqual(r, expectedInputs) {
		t.Errorf("Expected execlog %v\n\tgot %v", expectedInputs, r)
	}
}

func TestParseRunfilesManifest(t *testing.T) {
	// A small subset of a runfiles manifest copied from a real build
	runfilesManifest := `
_main/README.md /Users/me/dev/repo/README.md
_main/dev_/dev /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/dev_/dev
_main/dev_config.json /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/dev_config.json
_main/dev_node_bin/node /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/dev_node_bin/node
_main/mylib/index.js /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/mylib/index.js
_main/mylib/node_modules/chalk ../../node_modules/.aspect_rules_js/chalk@4.1.2/node_modules/chalk
_main/mylib/package.json /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/mylib/package.json
_main/node_modules/.aspect_rules_js/@bazel+ibazel@0.16.2/node_modules/@bazel/ibazel /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@bazel+ibazel@0.16.2/node_modules/@bazel/ibazel
_main/node_modules/.aspect_rules_js/@discoveryjs+json-ext@0.5.7/node_modules/@discoveryjs/json-ext /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@discoveryjs+json-ext@0.5.7/node_modules/@discoveryjs/json-ext
_main/node_modules/.aspect_rules_js/@mycorp+mylib@0.0.0/node_modules/@mycorp/mylib ../../../../../mylib
_main/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg
_repo_mapping /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/dev_/dev.repo_mapping
aspect_rules_js~/js/private/js_run_devserver.mjs /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main/bazel-out/darwin_arm64-fastbuild/bin/external/aspect_rules_js~/js/private/js_run_devserver.mjs
aspect_rules_js~/js/private/node-patches/fs.cjs /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/external/aspect_rules_js~/js/private/node-patches/fs.cjs
aspect_rules_js~/js/private/node-patches/register.cjs /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/external/aspect_rules_js~/js/private/node-patches/register.cjs
rules_nodejs~~node~nodejs_darwin_arm64/bin/nodejs/bin/node /private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/external/rules_nodejs~~node~nodejs_darwin_arm64/bin/nodejs/bin/node
`

	r, err := parseRunfilesManifest(strings.NewReader(strings.TrimSpace(runfilesManifest)), "/Users/me/dev/repo", "/private/var/tmp/_bazel_user/3c4e0d2fa783c7ab87494f9a6e5ea2c3/execroot/_main")
	if err != nil {
		t.Errorf("Failed to parse exec log: %v", err)
	}

	if len(r.runfiles) != 16 {
		t.Errorf("Expected 14 runfiles, got %d", len(r.runfiles))
	}

	// 1st-party node_modules
	mylib, mylibExists := r.runfiles["_main/node_modules/.aspect_rules_js/@mycorp+mylib@0.0.0/node_modules/@mycorp/mylib"]
	if !mylibExists {
		t.Errorf("Expected runfile for mylib directory to exist: %v", r.runfiles)
	}
	if mylib.is_source || mylib.is_external || !mylib.is_symlink {
		t.Errorf("Expected mylib to be a symlink, got is_source=%v, is_external=%v, is_symlink=%v", mylib.is_source, mylib.is_external, mylib.is_symlink)
	}

	// Content of that 1st-party package
	if _, mylibContentExists := r.runfiles["_main/mylib/index.js"]; !mylibContentExists {
		t.Errorf("Expected runfile for mylib content to exist: %v", r.runfiles)
	}
	if r.runfilesOriginMapping["bazel-out/darwin_arm64-fastbuild/bin/mylib/index.js"] != "_main/mylib/index.js" {
		t.Errorf("Expected bazel-out/darwin_arm64-fastbuild/bin/mylib/index.js to map to _main/mylib/index.js, got %s", r.runfilesOriginMapping["mylib/index.js"])
	}

	// node_modules of 1st-party packages
	if _, mylibDepExists := r.runfiles["_main/mylib/node_modules/chalk"]; !mylibDepExists {
		t.Errorf("Expected runfile for mylib/node_modules/chalk to exist: %v", r.runfiles)
	}

	// Source files
	if !r.runfiles["_main/README.md"].is_source || r.runfilesOriginMapping["README.md"] != "_main/README.md" {
		t.Errorf("Expected source mappings")
	}

	// External files
	if !r.runfiles["rules_nodejs~~node~nodejs_darwin_arm64/bin/nodejs/bin/node"].is_external {
		t.Errorf("Expected external mappings")
	}
}
