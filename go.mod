module aspect.build/cli

go 1.20

require (
	github.com/Masterminds/semver/v3 v3.2.1
	github.com/alphadose/haxmap v1.3.0
	github.com/bazel-contrib/rules_jvm v0.17.1-0.20230814153054-0ce5d051291b
	github.com/bazelbuild/bazel-gazelle v0.34.0
	github.com/bazelbuild/bazelisk v1.17.0
	github.com/bazelbuild/buildtools v0.0.0-20231017121127-23aa65d4e117
	github.com/bmatcuk/doublestar/v4 v4.6.1
	github.com/emirpasic/gods v1.18.1
	github.com/fatih/color v1.16.0
	github.com/golang/mock v1.6.0
	github.com/golang/protobuf v1.5.3
	github.com/hashicorp/go-hclog v1.5.0
	github.com/hashicorp/go-plugin v1.6.0
	github.com/manifoldco/promptui v0.9.0
	github.com/mattn/go-isatty v0.0.20
	github.com/mitchellh/go-homedir v1.1.0
	github.com/msolo/jsonr v0.0.0-20231023064044-62fbfc3a0313
	github.com/onsi/gomega v1.27.8
	github.com/pkg/browser v0.0.0-20210911075715-681adbf594b8
	github.com/pmezard/go-difflib v1.0.0
	github.com/rogpeppe/go-internal v1.10.0
	github.com/rs/zerolog v1.29.1
	github.com/sabhiram/go-gitignore v0.0.0-20210923224102-525f6e181f06
	github.com/smacker/go-tree-sitter v0.0.0-20230501083651-a7d92773b3aa
	github.com/spf13/cobra v1.7.0
	github.com/spf13/pflag v1.0.5
	github.com/spf13/viper v1.16.0
	github.com/tejzpr/ordered-concurrently/v3 v3.0.1
	github.com/twmb/murmur3 v1.1.8
	github.com/yargevad/filepathx v1.0.0
	go.starlark.net v0.0.0-20211203141949-70c0e40ae128
	golang.org/x/exp v0.0.0-20230713183714-613f0c0eb8a1
	golang.org/x/sync v0.4.0
	google.golang.org/genproto v0.0.0-20231120223509-83a465c0220f
	google.golang.org/grpc v1.59.0
	google.golang.org/protobuf v1.31.0
	gopkg.in/yaml.v3 v3.0.1
	sigs.k8s.io/yaml v1.4.0
)

replace github.com/smacker/go-tree-sitter v0.0.0-20230501083651-a7d92773b3aa => github.com/aspect-forks/go-tree-sitter v0.0.0-20230720070738-0d0a9f78d8f8

require (
	github.com/bazelbuild/rules_go v0.42.0 // indirect
	github.com/bgentry/go-netrc v0.0.0-20140422174119-9fd32a8b3d3d // indirect
	github.com/chzyer/readline v1.5.1 // indirect
	github.com/cpuguy83/go-md2man/v2 v2.0.2 // indirect
	github.com/fsnotify/fsnotify v1.7.0 // indirect
	github.com/google/btree v1.1.2 // indirect
	github.com/google/go-cmp v0.6.0 // indirect
	github.com/hashicorp/go-version v1.6.0 // indirect
	github.com/hashicorp/hcl v1.0.0 // indirect
	github.com/hashicorp/yamux v0.1.1 // indirect
	github.com/inconshreveable/mousetrap v1.1.0 // indirect
	github.com/magiconair/properties v1.8.7 // indirect
	github.com/mattn/go-colorable v0.1.13 // indirect
	github.com/mitchellh/go-testing-interface v1.14.1 // indirect
	github.com/mitchellh/mapstructure v1.5.0 // indirect
	github.com/oklog/run v1.1.0 // indirect
	github.com/pelletier/go-toml/v2 v2.0.8 // indirect
	github.com/russross/blackfriday/v2 v2.1.0 // indirect
	github.com/spf13/afero v1.9.5 // indirect
	github.com/spf13/cast v1.5.1 // indirect
	github.com/spf13/jwalterweatherman v1.1.0 // indirect
	github.com/subosito/gotenv v1.4.2 // indirect
	golang.org/x/mod v0.13.0 // indirect
	golang.org/x/net v0.17.0 // indirect
	golang.org/x/sys v0.14.0 // indirect
	golang.org/x/text v0.13.0 // indirect
	golang.org/x/tools/go/vcs v0.1.0-deprecated // indirect
	google.golang.org/genproto/googleapis/api v0.0.0-20231106174013-bbf56f31fb17 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20231106174013-bbf56f31fb17 // indirect
	gopkg.in/check.v1 v1.0.0-20201130134442-10cb98267c6c // indirect
	gopkg.in/ini.v1 v1.67.0 // indirect
)
