package plugin

/**
 * Starlark wrappers/interfaces/implementations in order for aspect-configure starzelle
 * plugins to interact with the aspect-configure plugin host.
 */

import (
	"fmt"
	"maps"
	"slices"

	starUtils "github.com/aspect-build/aspect-cli/gazelle/common/starlark/utils"
	"go.starlark.net/starlark"
)

// ---------------- PropertyValues
var _ starlark.Value = (*PropertyValues)(nil)
var _ starlark.Mapping = (*PropertyValues)(nil)

func (p PropertyValues) String() string {
	return fmt.Sprintf("PropertyValues{values: %v}", p.values)
}
func (p PropertyValues) Type() string         { return "PropertyValues" }
func (p PropertyValues) Freeze()              {}
func (p PropertyValues) Truth() starlark.Bool { return starlark.True }
func (p PropertyValues) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", p.Type())
}
func (p PropertyValues) Get(k starlark.Value) (v starlark.Value, found bool, err error) {
	if k.Type() != "string" {
		return nil, false, fmt.Errorf("invalid key type, expected string")
	}
	key := k.(starlark.String).GoString()
	r, found := p.values[key]

	if !found {
		return nil, false, fmt.Errorf("no property named: %s", key)
	}

	return starUtils.Write(r), true, nil
}

// ---------------- PrepareContext

var _ starlark.Value = (*PrepareContext)(nil)
var _ starlark.HasAttrs = (*PrepareContext)(nil)

func (ctx PrepareContext) String() string {
	return fmt.Sprintf("PrepareContext{repo_name: %q, rel: %q, properties: %v}", ctx.RepoName, ctx.Rel, ctx.Properties)
}
func (ctx PrepareContext) Type() string         { return "PrepareContext" }
func (ctx PrepareContext) Freeze()              {}
func (ctx PrepareContext) Truth() starlark.Bool { return starlark.True }
func (ctx PrepareContext) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", ctx.Type())
}

func (ctx PrepareContext) Attr(name string) (starlark.Value, error) {
	switch name {
	case "repo_name":
		return starlark.String(ctx.RepoName), nil
	case "rel":
		return starlark.String(ctx.Rel), nil
	case "properties":
		return ctx.Properties, nil
	}

	return nil, fmt.Errorf("no such attribute: %s on %s", name, ctx.Type())
}
func (ctx PrepareContext) AttrNames() []string {
	return []string{"repo_name", "rel", "properties"}
}

// ---------------- DeclareTargetsContext

const DeclareTargetsContextDefaultGroup = "default"

var _ starlark.Value = (*DeclareTargetsContext)(nil)
var _ starlark.HasAttrs = (*DeclareTargetsContext)(nil)

func (ctx DeclareTargetsContext) Attr(name string) (starlark.Value, error) {
	switch name {
	case "sources":
		return ctx.Sources, nil
	case "targets":
		return ctx.Targets.(*declareTargetActionsImpl), nil
	case "add_symbol":
		return contextAddSymbol.BindReceiver(ctx), nil
	}

	return ctx.PrepareContext.Attr(name)
}
func (ctx DeclareTargetsContext) String() string {
	return fmt.Sprintf("DeclareTargetsContext{PrepareContext: %v, sources: %v, targets: %v}", ctx.PrepareContext, ctx.Sources, ctx.Targets)
}
func (ctx DeclareTargetsContext) AttrNames() []string {
	return []string{"repo_name", "rel", "properties", "sources", "targets"}
}
func (ctx DeclareTargetsContext) Type() string { return "DeclareTargetsContext" }

// ---------------- TargetSourceList
var _ starlark.Value = (*TargetSourceList)(nil)

func (t TargetSourceList) Freeze() {
}
func (t TargetSourceList) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", t.Type())
}
func (t TargetSourceList) String() string {
	return fmt.Sprintf("TargetSourceList(%v)", len(t))
}
func (t TargetSourceList) Truth() starlark.Bool {
	return t.Len() > 0
}
func (t TargetSourceList) Type() string {
	return "TargetSourceList"
}

var _ starlark.Sequence = (*TargetSourceList)(nil)

func (t TargetSourceList) Iterate() starlark.Iterator {
	return &targetSourceListIterator{t: t}
}
func (t TargetSourceList) Len() int {
	return len(t)
}

type targetSourceListIterator struct {
	t TargetSourceList
	i int
}

func (t *targetSourceListIterator) Done() {}
func (t *targetSourceListIterator) Next(p *starlark.Value) bool {
	if t.i >= len(t.t) {
		return false
	}
	*p = t.t[t.i]
	t.i++
	return true
}

var _ starlark.Iterator = (*targetSourceListIterator)(nil)

var _ starlark.Indexable = (*TargetSourceList)(nil)

func (t TargetSourceList) Index(i int) starlark.Value {
	return t[i]
}

// ---------------- TargetSources
var _ starlark.Value = (*TargetSources)(nil)

func (c TargetSources) String() string {
	return fmt.Sprintf("DeclareTargetsContext.Sources{%v}", maps.Keys(c))
}
func (c TargetSources) Type() string { return "DeclareTargetsContext.Sources" }
func (c TargetSources) Freeze()      {}
func (c TargetSources) Truth() starlark.Bool {
	// Treat empty sources as falsy
	for _, groupSrcs := range c {
		if len(groupSrcs) > 0 {
			return starlark.True
		}
	}
	return starlark.False
}
func (c TargetSources) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", c.Type())
}

// Can fetch sources by group name
var _ starlark.HasAttrs = (*TargetSources)(nil)

func (c TargetSources) Attr(name string) (starlark.Value, error) {
	if groupSrcs, ok := c[name]; ok {
		return groupSrcs, nil
	}

	return nil, fmt.Errorf("no source group: %q, known groups: %v", name, c.AttrNames())
}
func (c TargetSources) AttrNames() []string {
	// TODO: exclude plugin.DeclareTargetsContextDefaultGroup
	return slices.Collect(maps.Keys(c))
}

// Iterator/array operations delegate to the default group
var _ starlark.Sequence = (*TargetSources)(nil)
var _ starlark.Indexable = (*TargetSources)(nil)

func (ts TargetSources) Len() int {
	return ts[DeclareTargetsContextDefaultGroup].Len()
}
func (ts TargetSources) Index(i int) starlark.Value {
	return ts[DeclareTargetsContextDefaultGroup].Index(i)
}
func (ts TargetSources) Iterate() starlark.Iterator {
	return ts[DeclareTargetsContextDefaultGroup].Iterate()
}

// ---------------- declareTargetActionsImpl

var _ starlark.Value = (*declareTargetActionsImpl)(nil)
var _ starlark.HasAttrs = (*declareTargetActionsImpl)(nil)

func (a *declareTargetActionsImpl) String() string {
	return fmt.Sprintf("declareTargetActionsImpl{%v}", a.actions)
}
func (a *declareTargetActionsImpl) Type() string         { return "declareTargetActionsImpl" }
func (a *declareTargetActionsImpl) Freeze()              {}
func (a *declareTargetActionsImpl) Truth() starlark.Bool { return starlark.True }
func (a *declareTargetActionsImpl) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", a.Type())
}
func (ai *declareTargetActionsImpl) Attr(name string) (starlark.Value, error) {
	switch name {
	case "add":
		return declareTargetAdd.BindReceiver(ai), nil
	case "remove":
		return declareTargetRemove.BindReceiver(ai), nil
	}

	return nil, fmt.Errorf("no such attribute: %s on %s", name, ai.Type())
}
func (*declareTargetActionsImpl) AttrNames() []string {
	return []string{"add", "remove"}
}

var declareTargetAdd = starlark.NewBuiltin("add", addTarget)

func addTarget(thread *starlark.Thread, fn *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var starName starlark.String
	var starKind starlark.String
	var starAttrs starlark.Mapping
	var starSymbols starlark.Value
	err := starlark.UnpackArgs(
		fn.Name(),
		args,
		kwargs,
		"name", &starName,
		"kind", &starKind,
		"attrs??", &starAttrs,
		"symbols??", &starSymbols,
	)
	if err != nil {
		return nil, err
	}

	// TODO: don't create new clones of map/arrays every time

	var attrs map[string]interface{}
	if starAttrs != nil {
		attrs = starUtils.ReadMap2(starAttrs, readTargetAttributeValue)
	}

	var symbols []Symbol
	if starSymbols != nil {
		symbols = starUtils.ReadList(starSymbols, readSymbol)
	}

	ai := fn.Receiver().(*declareTargetActionsImpl)
	ai.Add(TargetDeclaration{
		Name:    starName.GoString(),
		Kind:    starKind.GoString(),
		Attrs:   attrs,
		Symbols: symbols,
	})

	return starlark.None, nil
}

var declareTargetRemove = starlark.NewBuiltin("remove", removeTarget)

func removeTarget(thread *starlark.Thread, fn *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var name, kind starlark.String
	if err := starlark.UnpackArgs(fn.Name(), args, kwargs, "name", &name, "kind??", &kind); err != nil {
		return nil, err
	}

	ai := fn.Receiver().(*declareTargetActionsImpl)
	ai.Remove(name.GoString(), kind.GoString())
	return starlark.None, nil
}

// ---------------- TargetSource

var _ starlark.Value = (*TargetSource)(nil)
var _ starlark.HasAttrs = (*TargetSource)(nil)

func (ts TargetSource) String() string {
	return fmt.Sprintf("TargetSource{path: %q, query_results: %v}", ts.Path, ts.QueryResults)
}
func (TargetSource) Freeze() {}
func (TargetSource) Truth() starlark.Bool {
	return starlark.True
}
func (TargetSource) Type() string {
	return "TargetSource"
}
func (ts TargetSource) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", ts.Type())
}

func (ctx TargetSource) Attr(name string) (starlark.Value, error) {
	switch name {
	case "path":
		return starlark.String(ctx.Path), nil
	case "query_results":
		return ctx.QueryResults, nil
	}

	return nil, fmt.Errorf("no such attribute: %s on %s", name, ctx.Type())
}
func (ctx TargetSource) AttrNames() []string {
	return []string{"path", "query_results"}
}

// ---------------- Property

var _ starlark.Value = (*Property)(nil)
var _ starlark.HasAttrs = (*Property)(nil)

func (p Property) String() string {
	return fmt.Sprintf("Property{name: %q, type: %q, default: %q}", p.Name, p.PropertyType, p.Default)
}
func (p Property) Type() string         { return "Property" }
func (p Property) Freeze()              {}
func (p Property) Truth() starlark.Bool { return starlark.True }
func (p Property) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", p.Type())
}
func (p Property) Attr(name string) (starlark.Value, error) {
	switch name {
	case "name":
		return starUtils.Write(p.Name), nil
	case "type":
		return starUtils.Write(p.PropertyType), nil
	case "default":
		return starUtils.Write(p.Default), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (p Property) AttrNames() []string {
	return []string{"name", "type", "default"}
}

// ---------------- PrepareResult

var _ starlark.Value = (*PrepareResult)(nil)
var _ starlark.HasAttrs = (*PrepareResult)(nil)

func (r PrepareResult) String() string {
	return fmt.Sprintf("PrepareResult{sources: %v, queries: %v}", r.Sources, r.Queries)
}
func (r PrepareResult) Type() string         { return "PrepareResult" }
func (r PrepareResult) Freeze()              {}
func (r PrepareResult) Truth() starlark.Bool { return starlark.True }
func (r PrepareResult) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", r.Type())
}
func (r PrepareResult) Attr(name string) (starlark.Value, error) {
	switch name {
	case "sources":
		return starUtils.Write(r.Sources), nil
	case "queries":
		return starUtils.Write(r.Queries), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (r PrepareResult) AttrNames() []string {
	return []string{"sources", "queries"}
}

// ---------------- SourceExtensionsFilter

var _ starlark.Value = (*SourceExtensionsFilter)(nil)
var _ starlark.HasAttrs = (*SourceExtensionsFilter)(nil)

func (r SourceExtensionsFilter) String() string {
	return fmt.Sprintf("SourceExtensionsFilter{Extensions: %v}", r.Extensions)
}
func (r SourceExtensionsFilter) Type() string         { return "SourceExtensionsFilter" }
func (r SourceExtensionsFilter) Freeze()              {}
func (r SourceExtensionsFilter) Truth() starlark.Bool { return starlark.True }
func (r SourceExtensionsFilter) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", r.Type())
}
func (r SourceExtensionsFilter) Attr(name string) (starlark.Value, error) {
	switch name {
	case "extensions":
		return starUtils.Write(r.Extensions), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (r SourceExtensionsFilter) AttrNames() []string {
	return []string{"extensions"}
}

// ---------------- SourceFileFilter

var _ starlark.Value = (*SourceGlobFilter)(nil)
var _ starlark.HasAttrs = (*SourceGlobFilter)(nil)

func (r SourceGlobFilter) String() string {
	return fmt.Sprintf("SourceGlobFilter{Globs: %v}", r.Globs)
}
func (r SourceGlobFilter) Type() string         { return "SourceGlobFilter" }
func (r SourceGlobFilter) Freeze()              {}
func (r SourceGlobFilter) Truth() starlark.Bool { return starlark.True }
func (r SourceGlobFilter) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", r.Type())
}
func (r SourceGlobFilter) Attr(name string) (starlark.Value, error) {
	switch name {
	case "globs":
		return starUtils.Write(r.Globs), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (r SourceGlobFilter) AttrNames() []string {
	return []string{"globs"}
}

// ---------------- SourceFileFilter

var _ starlark.Value = (*SourceFileFilter)(nil)
var _ starlark.HasAttrs = (*SourceFileFilter)(nil)

func (r SourceFileFilter) String() string {
	return fmt.Sprintf("SourceFileFilter{Files: %v}", r.Files)
}
func (r SourceFileFilter) Type() string         { return "SourceFileFilter" }
func (r SourceFileFilter) Freeze()              {}
func (r SourceFileFilter) Truth() starlark.Bool { return starlark.True }
func (r SourceFileFilter) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", r.Type())
}
func (r SourceFileFilter) Attr(name string) (starlark.Value, error) {
	switch name {
	case "files":
		return starUtils.Write(r.Files), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (r SourceFileFilter) AttrNames() []string {
	return []string{"files"}
}

// ---------------- AnalyzeContext

var _ starlark.Value = (*AnalyzeContext)(nil)
var _ starlark.HasAttrs = (*AnalyzeContext)(nil)

func (a AnalyzeContext) Attr(name string) (starlark.Value, error) {
	switch name {
	case "source":
		return a.Source, nil
	case "add_symbol":
		return contextAddSymbol.BindReceiver(a), nil
	}
	return a.PrepareContext.Attr(name)
}

func (a AnalyzeContext) AttrNames() []string {
	return []string{"repo_name", "rel", "properties", "source", "add_symbol"}
}
func (a AnalyzeContext) Freeze() {}
func (a AnalyzeContext) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", a.Type())
}
func (a AnalyzeContext) String() string {
	return fmt.Sprintf("AnalyzeContext{source: %v}", a.Source)
}
func (a AnalyzeContext) Truth() starlark.Bool { return starlark.True }
func (a AnalyzeContext) Type() string         { return "AnalyzeContext" }

var contextAddSymbol = starlark.NewBuiltin("add_symbol", addSymbol)

func addSymbol(thread *starlark.Thread, fn *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var id, provider_type string
	var label Label
	err := starlark.UnpackArgs(
		"add_symbol", args, kwargs,
		"id", &id,
		"provider_type", &provider_type,
		"label", &label,
	)
	if err != nil {
		return nil, err
	}

	ctx := fn.Receiver()

	if actx, isACtx := ctx.(AnalyzeContext); isACtx {
		actx.AddSymbol(label, Symbol{
			Id:       id,
			Provider: provider_type,
		})
	} else {
		ctx.(DeclareTargetsContext).AddSymbol(label, Symbol{
			Id:       id,
			Provider: provider_type,
		})
	}

	return starlark.None, nil
}

// ---------------- Gazelle Label

var _ starlark.Value = (*Label)(nil)
var _ starlark.HasAttrs = (*Label)(nil)

func (l Label) Attr(name string) (starlark.Value, error) {
	switch name {
	case "repo":
		return starlark.String(l.Repo), nil
	case "pkg":
		return starlark.String(l.Pkg), nil
	case "name":
		return starlark.String(l.Name), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}

func (l Label) AttrNames() []string {
	return []string{"repo", "pkg", "name"}
}

func (l Label) String() string {
	return fmt.Sprintf("Label{repo: %q, pkg: %q, name: %q}", l.Repo, l.Pkg, l.Name)
}
func (l Label) Type() string         { return "Label" }
func (l Label) Freeze()              {}
func (l Label) Truth() starlark.Bool { return starlark.True }
func (l Label) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", l.Type())
}
