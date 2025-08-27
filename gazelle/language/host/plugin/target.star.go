package plugin

import (
	"fmt"

	starUtils "github.com/aspect-build/aspect-cli/gazelle/common/starlark/utils"

	"go.starlark.net/starlark"
)

// ---------------- Symbol

var _ starlark.Value = (*Symbol)(nil)
var _ starlark.HasAttrs = (*Symbol)(nil)

func (s Symbol) String() string {
	return fmt.Sprintf("Symbol{id: %q, provider: %q}", s.Id, s.Provider)
}
func (s Symbol) Type() string         { return "Symbol" }
func (s Symbol) Freeze()              {}
func (s Symbol) Truth() starlark.Bool { return starlark.True }
func (s Symbol) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", s.Type())
}

func (s Symbol) Attr(name string) (starlark.Value, error) {
	switch name {
	case "id":
		return starlark.String(s.Id), nil
	case "provider":
		return starlark.String(s.Provider), nil
	}

	return nil, fmt.Errorf("no such attribute: %s on %s", name, s.Type())
}
func (s Symbol) AttrNames() []string {
	return []string{"id", "provider"}
}

// ---------------- TargetImport

var _ starlark.Value = (*TargetImport)(nil)
var _ starlark.HasAttrs = (*TargetImport)(nil)

func (ti TargetImport) String() string {
	return fmt.Sprintf("TargetImport{id: %q, provider: %q from: %q}", ti.Id, ti.Provider, ti.From)
}
func (ti TargetImport) Type() string         { return "TargetImport" }
func (ti TargetImport) Freeze()              {}
func (ti TargetImport) Truth() starlark.Bool { return starlark.True }
func (ti TargetImport) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", ti.Type())
}

func (ti TargetImport) Attr(name string) (starlark.Value, error) {
	switch name {
	case "id":
		return starlark.String(ti.Id), nil
	case "provider":
		return starlark.String(ti.Provider), nil
	case "from":
		return starlark.String(ti.From), nil
	case "optional":
		return starlark.Bool(ti.Optional), nil
	}

	return nil, fmt.Errorf("no such attribute: %s on %s", name, ti.Type())
}
func (ti TargetImport) AttrNames() []string {
	return []string{"id", "provider", "from", "optional"}
}

// ---------------- TargetSymbol

var _ starlark.Value = (*TargetSymbol)(nil)
var _ starlark.HasAttrs = (*TargetSymbol)(nil)

func (te TargetSymbol) String() string {
	return fmt.Sprintf("TargetSymbol{id: %q, provider: %q, label: %q}", te.Id, te.Provider, te.Label)
}
func (te TargetSymbol) Type() string         { return "TargetSymbol" }
func (te TargetSymbol) Freeze()              {}
func (te TargetSymbol) Truth() starlark.Bool { return starlark.True }
func (te TargetSymbol) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", te.Type())
}

func (te TargetSymbol) Attr(name string) (starlark.Value, error) {
	switch name {
	case "id":
		return starlark.String(te.Id), nil
	case "provider":
		return starlark.String(te.Provider), nil
	case "label":
		return te.Label, nil
	}

	return nil, fmt.Errorf("no such attribute: %s on %s", name, te.Type())
}
func (te TargetSymbol) AttrNames() []string {
	return []string{"id", "provider", "label"}
}

// ---------------- utils

func readTargetImport(v starlark.Value) TargetImport {
	return v.(TargetImport)
}

func readSymbol(v starlark.Value) Symbol {
	return v.(Symbol)
}

func readLabel(v starlark.Value) Label {
	return v.(Label)
}

func readTargetAttributeValue(v starlark.Value) interface{} {
	switch v := v.(type) {
	case TargetImport:
		return readTargetImport(v)
	case Label:
		return readLabel(v)
	case TargetSource:
		return v.Path
	}

	return starUtils.ReadRecurse(v, readTargetAttributeValue)
}
