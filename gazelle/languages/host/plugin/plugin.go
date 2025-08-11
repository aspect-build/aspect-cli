package plugin

import (
	"encoding/gob"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
)

type PluginId = string

type PluginHost interface {
	AddKind(k RuleKind)
	AddPlugin(plugin Plugin)
}

// TODO: change the interface into a factory method (at least in starzelle)
type Plugin interface {
	// Static plugin metadata
	Name() PluginId
	Properties() map[string]Property

	// Prepare for generating targets
	Prepare(ctx PrepareContext) PrepareResult
	Analyze(ctx AnalyzeContext) error
	DeclareTargets(ctx DeclareTargetsContext) DeclareTargetsResult
}

type PropertyType = string

const (
	PropertyType_String  PropertyType = "string"
	PropertyType_Strings PropertyType = "[]string"
	PropertyType_Bool    PropertyType = "bool"
	PropertyType_Number  PropertyType = "number"
)

type RuleKind struct {
	KindInfo
	Name string
	From string
}

// Subset of the bazel-gazelle rule.KindInfo. See bazel-gazelle for details.
type KindInfo struct {
	// MatchAny is true if a rule of this kind may be matched with any rule
	// of the same kind, regardless of attributes, if exactly one rule is
	// present a build file.
	MatchAny bool

	// MatchAttrs is a list of attributes used in matching. For example,
	// for go_library, this list contains "importpath". Attributes are matched
	// in order.
	MatchAttrs []string

	// NonEmptyAttrs is a set of attributes that, if present, disqualify a rule
	// from being deleted after merge.
	NonEmptyAttrs []string

	// MergeableAttrs is a set of attributes that should be merged before
	// dependency resolution. For example "srcs" are often merged before resolution
	// to compute the full set of sources for a target before resolving dependencies.
	MergeableAttrs []string

	// ResolveAttrs is a set of attributes that should be merged after
	// dependency resolution. For example "deps" are often merged after resolution.
	ResolveAttrs []string
}

// Properties an extension can be configured
type Property struct {
	Name         string // TODO: drop because it's always specified in a map[Name]?
	PropertyType PropertyType
	Default      interface{}
}

type PropertyValues struct {
	values map[string]interface{}
}

func NewPropertyValues() PropertyValues {
	return PropertyValues{
		values: make(map[string]interface{}),
	}
}

func (pv PropertyValues) Add(name string, value interface{}) {
	pv.values[name] = value
}

// The context for an extension to prepare for generating targets.
type PrepareContext struct {
	RepoName   string
	Rel        string
	Properties PropertyValues
}

// The result of an extension preparing for generating targets.
//
// Queries are mapped by file extension and will be executed against all
// matching extensions.
//
// Example:
//
//	 PrepareResult {
//			Extensions: [".java"],
//			Queries: {
//				"imports": {
//					"Type": "string|strings|exists",
//					"Extensions": ["*.java"],
//					"Query": "(import_list)",
//				},
//			},
//	 }
type PrepareResult struct {
	Sources map[string][]SourceFilter
	Queries NamedQueries
}

type SourceFilter interface {
	Match(p string) bool
}

var _ SourceFilter = (*SourceGlobFilter)(nil)

type SourceGlobFilter struct {
	Globs []string
}

func (f SourceGlobFilter) Match(p string) bool {
	for _, glob := range f.Globs {
		if doublestar.MatchUnvalidated(glob, p) {
			return true
		}
	}
	return false
}

var _ SourceFilter = (*SourceExtensionsFilter)(nil)

type SourceExtensionsFilter struct {
	Extensions []string
}

func (f SourceExtensionsFilter) Match(p string) bool {
	for _, ext := range f.Extensions {
		if strings.HasSuffix(p, ext) {
			return true
		}
	}
	return false
}

var _ SourceFilter = (*SourceFileFilter)(nil)

type SourceFileFilter struct {
	Files []string
}

func (sf SourceFileFilter) Match(p string) bool {
	for _, f := range sf.Files {
		if p == f {
			return true
		}
	}
	return false
}

type Label struct {
	Repo, Pkg, Name string
}

type AnalyzeContext struct {
	PrepareContext
	Source   *TargetSource
	database *Database
}

func (a AnalyzeContext) AddSymbol(label Label, symbol Symbol) {
	a.database.AddSymbol(label, symbol)
}

func NewAnalyzeContext(prep PrepareContext, source *TargetSource, database *Database) AnalyzeContext {
	return AnalyzeContext{
		PrepareContext: prep,
		Source:         source,
		database:       database,
	}
}

type TargetSources map[string]TargetSourceList
type TargetSourceList []TargetSource

// The context for an extension to generate targets.
//
// Queries results are mapped by file extension, each containing a map of
// query name to result.
type DeclareTargetsContext struct {
	PrepareContext
	Sources  TargetSources
	Targets  DeclareTargetActions
	database *Database
}

func (d DeclareTargetsContext) AddSymbol(label Label, symbol Symbol) {
	d.database.AddSymbol(label, symbol)
}

func NewDeclareTargetsContext(prep PrepareContext, sources TargetSources, targets DeclareTargetActions, database *Database) DeclareTargetsContext {
	return DeclareTargetsContext{
		PrepareContext: prep,
		Sources:        sources,
		Targets:        targets,
		database:       database,
	}
}

type DeclareTargetActions interface {
	Add(target TargetDeclaration)
	Remove(name, kind string)
	Actions() []TargetAction
}

var _ DeclareTargetActions = (*declareTargetActionsImpl)(nil)

type declareTargetActionsImpl struct {
	actions []TargetAction
}

func NewDeclareTargetActions() DeclareTargetActions {
	return &declareTargetActionsImpl{
		actions: make([]TargetAction, 0),
	}
}
func (ctx *declareTargetActionsImpl) Actions() []TargetAction {
	return ctx.actions
}
func (ctx *declareTargetActionsImpl) Add(t TargetDeclaration) {
	ctx.actions = append(ctx.actions, AddTargetAction{
		TargetDeclaration: t,
	})
}
func (ctx *declareTargetActionsImpl) Remove(name, kind string) {
	ctx.actions = append(ctx.actions, RemoveTargetAction{
		Name: name,
		Kind: kind,
	})
}

// The result of declaring targets
type DeclareTargetsResult struct {
	Actions []TargetAction
}

type TargetSource struct {
	Path         string
	QueryResults QueryResults
}

func init() {
	// TODO: don't expose 'gob' cache serialization here
	gob.Register(QueryResults{})
	gob.Register(QueryMatches{})
	gob.Register(QueryMatch{})
	gob.Register(QueryCapture{})
	gob.Register(QueryProcessorResult{})
}
