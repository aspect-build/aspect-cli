// Deprecated: use github.com/aspect-build/aspect-cli-legacy instead.
module github.com/aspect-build/aspect-cli

go 1.24.5

require (
	github.com/Masterminds/semver/v3 v3.4.0 // indirect
	github.com/alphadose/haxmap v1.4.1
	github.com/aspect-build/aspect-gazelle/common v0.0.0-20251009224809-e75afa1f8569
	github.com/aspect-build/aspect-gazelle/language/orion v0.0.0-20251009224809-e75afa1f8569
	github.com/aspect-build/aspect-gazelle/runner v0.0.0-20251009224809-e75afa1f8569
	github.com/bazelbuild/bazel-gazelle v0.45.1-0.20250924144014-2de7b829fef1
	github.com/bazelbuild/bazelisk v1.26.0 // NOTE: keep vendored code in sync
	github.com/bazelbuild/buildtools v0.0.0-20250926132224-6c4b75d79427
	github.com/bluekeyes/go-gitdiff v0.7.3
	github.com/bmatcuk/doublestar/v4 v4.9.1 // indirect
	github.com/emirpasic/gods v1.18.1 // indirect
	github.com/fatih/color v1.18.0
	github.com/go-git/go-git/v5 v5.16.2 // indirect
	github.com/golang/mock v1.7.0-rc.1
	github.com/golang/protobuf v1.5.4
	github.com/hashicorp/go-hclog v1.6.3
	github.com/hashicorp/go-plugin v1.6.1
	github.com/itchyny/gojq v0.12.18-0.20251005142832-e46d0344f209 // indirect
	github.com/manifoldco/promptui v0.9.0
	github.com/mattn/go-isatty v0.0.20
	github.com/mitchellh/go-homedir v1.1.0
	github.com/msolo/jsonr v0.0.0-20231023064044-62fbfc3a0313 // indirect
	github.com/onsi/gomega v1.38.2
	github.com/pkg/browser v0.0.0-20240102092130-5ac0b6a4141c
	github.com/pmezard/go-difflib v1.0.1-0.20181226105442-5d4384ee4fb2 // indirect
	github.com/reviewdog/errorformat v0.0.0-20240608101709-1d3280ed6bd4
	github.com/rs/zerolog v1.34.0
	github.com/smacker/go-tree-sitter v0.0.0-20240827094217-dd81d9e9be82 // indirect
	github.com/spf13/cobra v1.10.1
	github.com/spf13/pflag v1.0.10
	github.com/spf13/viper v1.20.1
	github.com/tejzpr/ordered-concurrently/v3 v3.0.1
	github.com/twmb/murmur3 v1.1.8
	go.starlark.net v0.0.0-20250717191651-336a4b3a6d1d // indirect
	golang.org/x/exp v0.0.0-20250718183923-645b1fa84792 // indirect
	golang.org/x/mod v0.28.0
	golang.org/x/sync v0.17.0
	golang.org/x/tools v0.37.0
	google.golang.org/genproto v0.0.0-20250603155806-513f23925822
	google.golang.org/grpc v1.73.0
	google.golang.org/protobuf v1.36.9
	gopkg.in/yaml.v3 v3.0.1 // indirect
	sigs.k8s.io/yaml v1.4.0
)

require (
	github.com/charmbracelet/huh v0.6.0
	github.com/creack/pty v1.1.24
	github.com/hay-kot/scaffold v0.6.2-0.20250317013600-8a6092d5e4ff
	github.com/reviewdog/reviewdog v0.17.4
	github.com/sourcegraph/go-diff v0.7.0
	go.opentelemetry.io/otel v1.37.0
	go.opentelemetry.io/otel/exporters/stdout/stdouttrace v1.37.0
	go.opentelemetry.io/otel/sdk v1.37.0
	go.opentelemetry.io/otel/trace v1.37.0
	golang.org/x/term v0.35.0
	google.golang.org/genproto/googleapis/api v0.0.0-20250603155806-513f23925822
	gotest.tools/v3 v3.5.1
)

require github.com/sagikazarmark/locafero v0.7.0 // indirect

require (
	dario.cat/mergo v1.0.2 // indirect
	github.com/EngFlow/gazelle_cc v0.1.0 // indirect
	github.com/Microsoft/go-winio v0.6.2 // indirect
	github.com/ProtonMail/go-crypto v1.1.6 // indirect
	github.com/a8m/envsubst v1.4.3 // indirect
	github.com/alecthomas/chroma/v2 v2.15.0 // indirect
	github.com/alecthomas/participle/v2 v2.1.4 // indirect
	github.com/aspect-build/aspect-gazelle/language/js v0.0.0-20251001194427-669d53ca49e3 // indirect
	github.com/atotto/clipboard v0.1.4 // indirect
	github.com/aymanbagabas/go-osc52/v2 v2.0.1 // indirect
	github.com/aymerick/douceur v0.2.0 // indirect
	github.com/bazel-contrib/rules_python/gazelle v0.0.0-20250921132421-38f2679fcc6c // indirect
	github.com/bgentry/go-netrc v0.0.0-20140422174119-9fd32a8b3d3d // indirect
	github.com/catppuccin/go v0.3.0 // indirect
	github.com/charmbracelet/bubbles v0.21.0 // indirect
	github.com/charmbracelet/bubbletea v1.3.6 // indirect
	github.com/charmbracelet/colorprofile v0.3.1 // indirect
	github.com/charmbracelet/glamour v0.10.0 // indirect
	github.com/charmbracelet/huh/spinner v0.0.0-20250714122654-40d2b68703eb // indirect
	github.com/charmbracelet/lipgloss v1.1.1-0.20250404203927-76690c660834 // indirect
	github.com/charmbracelet/x/ansi v0.9.3 // indirect
	github.com/charmbracelet/x/cellbuf v0.0.13 // indirect
	github.com/charmbracelet/x/exp/slice v0.0.0-20250720010745-3615766e35a0 // indirect
	github.com/charmbracelet/x/exp/strings v0.0.0-20250720010745-3615766e35a0 // indirect
	github.com/charmbracelet/x/term v0.2.1 // indirect
	github.com/chzyer/readline v1.5.1 // indirect
	github.com/cloudflare/circl v1.6.1 // indirect
	github.com/cpuguy83/go-md2man/v2 v2.0.6 // indirect
	github.com/cyphar/filepath-securejoin v0.4.1 // indirect
	github.com/dimchansky/utfbom v1.1.1 // indirect
	github.com/dlclark/regexp2 v1.11.5 // indirect
	github.com/dustin/go-humanize v1.0.1 // indirect
	github.com/elliotchance/orderedmap v1.8.0 // indirect
	github.com/erikgeiser/coninput v0.0.0-20211004153227-1c3628e74d0f // indirect
	github.com/fsnotify/fsnotify v1.8.0 // indirect
	github.com/gertd/go-pluralize v0.2.1 // indirect
	github.com/go-git/gcfg v1.5.1-0.20230307220236-3a3c6141e376 // indirect
	github.com/go-git/go-billy/v5 v5.6.2 // indirect
	github.com/go-ini/ini v1.67.0 // indirect
	github.com/go-logr/logr v1.4.3 // indirect
	github.com/go-logr/stdr v1.2.2 // indirect
	github.com/go-sprout/sprout v1.0.0 // indirect
	github.com/go-viper/mapstructure/v2 v2.2.1 // indirect
	github.com/goccy/go-json v0.10.5 // indirect
	github.com/goccy/go-yaml v1.18.0 // indirect
	github.com/gofrs/flock v0.12.1 // indirect
	github.com/golang/groupcache v0.0.0-20241129210726-2c02b8208cf8 // indirect
	github.com/google/go-cmp v0.7.0 // indirect
	github.com/google/uuid v1.6.0 // indirect
	github.com/gorilla/css v1.0.1 // indirect
	github.com/hashicorp/go-version v1.7.0 // indirect
	github.com/hashicorp/yamux v0.1.1 // indirect
	github.com/haya14busa/go-checkstyle v0.0.0-20170303121022-5e9d09f51fa1 // indirect
	github.com/haya14busa/go-sarif v0.0.0-20210102043135-e2c5fed2fa3d // indirect
	github.com/huandu/xstrings v1.5.0 // indirect
	github.com/inconshreveable/mousetrap v1.1.0 // indirect
	github.com/itchyny/timefmt-go v0.1.6 // indirect
	github.com/jbenet/go-context v0.0.0-20150711004518-d14ea06fba99 // indirect
	github.com/jinzhu/copier v0.4.0 // indirect
	github.com/kevinburke/ssh_config v1.2.0 // indirect
	github.com/lucasb-eyer/go-colorful v1.2.0 // indirect
	github.com/magiconair/properties v1.8.10 // indirect
	github.com/mattn/go-colorable v0.1.14 // indirect
	github.com/mattn/go-localereader v0.0.1 // indirect
	github.com/mattn/go-runewidth v0.0.16 // indirect
	github.com/microcosm-cc/bluemonday v1.0.27 // indirect
	github.com/mikefarah/yq/v4 v4.47.2 // indirect
	github.com/mitchellh/copystructure v1.2.0 // indirect
	github.com/mitchellh/go-testing-interface v1.14.1 // indirect
	github.com/mitchellh/hashstructure/v2 v2.0.2 // indirect
	github.com/mitchellh/reflectwalk v1.0.2 // indirect
	github.com/muesli/ansi v0.0.0-20230316100256-276c6243b2f6 // indirect
	github.com/muesli/cancelreader v0.2.2 // indirect
	github.com/muesli/reflow v0.3.0 // indirect
	github.com/muesli/termenv v0.16.0 // indirect
	github.com/oklog/run v1.1.0 // indirect
	github.com/pelletier/go-toml/v2 v2.2.4 // indirect
	github.com/pjbgf/sha1cd v0.4.0 // indirect
	github.com/psanford/memfs v0.0.0-20241019191636-4ef911798f9b // indirect
	github.com/rivo/uniseg v0.4.7 // indirect
	github.com/russross/blackfriday/v2 v2.1.0 // indirect; indirect	github.com/sagikazarmark/locafero v0.9.0 // indirect
	github.com/sahilm/fuzzy v0.1.1 // indirect
	github.com/sergi/go-diff v1.3.2-0.20230802210424-5b0b94c5c0d3 // indirect
	github.com/skeema/knownhosts v1.3.1 // indirect
	github.com/sourcegraph/conc v0.3.0 // indirect
	github.com/spf13/afero v1.12.0 // indirect
	github.com/spf13/cast v1.7.1 // indirect
	github.com/subosito/gotenv v1.6.0 // indirect
	github.com/urfave/cli/v2 v2.27.6 // indirect
	github.com/xanzy/ssh-agent v0.3.3 // indirect
	github.com/xo/terminfo v0.0.0-20220910002029-abceb7e1c41e // indirect
	github.com/xrash/smetrics v0.0.0-20240521201337-686a1a2994c1 // indirect
	github.com/yuin/goldmark v1.7.8 // indirect
	github.com/yuin/goldmark-emoji v1.0.5 // indirect
	github.com/yuin/gopher-lua v1.1.1 // indirect
	go.opentelemetry.io/auto/sdk v1.1.0 // indirect
	go.opentelemetry.io/otel/metric v1.37.0 // indirect
	go.uber.org/atomic v1.9.0 // indirect
	go.uber.org/multierr v1.9.0 // indirect
	go.yaml.in/yaml/v3 v3.0.4 // indirect
	golang.org/x/crypto v0.42.0 // indirect
	golang.org/x/net v0.44.0 // indirect
	golang.org/x/sys v0.36.0 // indirect
	golang.org/x/text v0.29.0 // indirect
	golang.org/x/tools/go/vcs v0.1.0-deprecated // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20250528174236-200df99c418a // indirect
	gopkg.in/op/go-logging.v1 v1.0.0-20160211212156-b2cb9fa56473 // indirect
	gopkg.in/warnings.v0 v0.1.2 // indirect
	gopkg.in/yaml.v2 v2.4.0 // indirect
)
