package plugin

import (
	"fmt"
	"maps"

	starUtils "github.com/aspect-build/aspect-cli/gazelle/common/starlark/utils"
	"go.starlark.net/starlark"
)

// ---------------- QueryCapture

var _ starlark.Mapping = (*QueryCapture)(nil)

func (q *QueryCapture) Get(k starlark.Value) (v starlark.Value, found bool, err error) {
	if k.Type() != "string" {
		return nil, false, fmt.Errorf("invalid key type, expected string")
	}
	key := k.(starlark.String).GoString()
	r, found := (*q)[key]

	if !found {
		return nil, false, fmt.Errorf("no capture named: %s", key)
	}
	return starlark.String(r), true, nil
}

func (q *QueryCapture) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", q.Type())
}

func (q *QueryCapture) Freeze() {}
func (q *QueryCapture) String() string {
	return fmt.Sprintf("QueryCapture{%v}", maps.Keys(*q))
}
func (q *QueryCapture) Truth() starlark.Bool { return starlark.True }
func (q *QueryCapture) Type() string         { return "QueryCapture" }

// ---------------- QueryMatch

var _ starlark.HasAttrs = (*QueryMatch)(nil)

func (q *QueryMatch) Attr(name string) (starlark.Value, error) {
	switch name {
	case "result":
		return starUtils.Write(q.Result), nil
	case "captures":
		return &q.Captures, nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (q *QueryMatch) AttrNames() []string {
	return []string{"result", "captures"}
}

func (q *QueryMatch) String() string {
	return fmt.Sprintf("QueryMatch(%v, captures: %v)", q.Result, q.Captures)
}
func (q *QueryMatch) Type() string {
	return "QueryMatch"
}
func (q *QueryMatch) Freeze()              {}
func (q *QueryMatch) Truth() starlark.Bool { return starlark.True }
func (q *QueryMatch) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", q.Type())
}

// ---------------- queryMatchIterator

type queryMatchIterator struct {
	m      QueryMatches
	cursor int
}

var _ starlark.Iterator = (*queryMatchIterator)(nil)

func (q *queryMatchIterator) Done() {
	q.cursor = 0
}

func (q *queryMatchIterator) Next(p *starlark.Value) bool {
	if q.cursor+1 > len(q.m) {
		return false
	}
	match := q.m[q.cursor]
	*p = &match
	q.cursor++
	return true
}

// ---------------- QueryMatches

var _ starlark.Value = (*QueryMatches)(nil)
var _ starlark.Iterable = (*QueryMatches)(nil)
var _ starlark.Indexable = (*QueryMatches)(nil)

func (q QueryMatches) Index(i int) starlark.Value {
	return &q[i]
}

func (q QueryMatches) Len() int {
	return len(q)
}

func (q QueryMatches) Freeze() {}

func (q QueryMatches) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", q.Type())
}

func (q QueryMatches) Iterate() starlark.Iterator {
	return &queryMatchIterator{m: q, cursor: 0}
}

func (q QueryMatches) String() string {
	return fmt.Sprintf("QueryMatches(%v)", len(q))
}

func (q QueryMatches) Truth() starlark.Bool {
	return starlark.True
}

func (q QueryMatches) Type() string {
	return "QueryMatches"
}

// ---------------- QueryDefinition

var _ starlark.Value = (*QueryDefinition)(nil)
var _ starlark.HasAttrs = (*QueryDefinition)(nil)

func (qd QueryDefinition) String() string {
	return fmt.Sprintf("QueryDefinition{filter: %v}", qd.Filter)
}
func (qd QueryDefinition) Type() string         { return "QueryDefinition" }
func (qd QueryDefinition) Freeze()              {}
func (qd QueryDefinition) Truth() starlark.Bool { return starlark.True }
func (qd QueryDefinition) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", qd.Type())
}
func (qd QueryDefinition) Attr(name string) (starlark.Value, error) {
	switch name {
	case "filter":
		return starUtils.Write(qd.Filter), nil
	case "params":
		return starUtils.Write(qd.Params), nil
	default:
		return nil, starlark.NoSuchAttrError(name)
	}
}
func (qd QueryDefinition) AttrNames() []string {
	return []string{"filter", "params"}
}

// ---------------- NamedQueries

var _ starlark.Value = (*NamedQueries)(nil)
var _ starlark.Mapping = (*NamedQueries)(nil)

func (nq NamedQueries) String() string {
	return fmt.Sprintf("NamedQueries(%v)", maps.Keys(nq))
}
func (nq NamedQueries) Type() string         { return "NamedQueries" }
func (nq NamedQueries) Freeze()              {}
func (nq NamedQueries) Truth() starlark.Bool { return starlark.True }
func (nq NamedQueries) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", nq.Type())
}

func (nq NamedQueries) Get(k starlark.Value) (v starlark.Value, found bool, err error) {
	if k.Type() != "string" {
		return nil, false, fmt.Errorf("invalid key type, expected string")
	}
	key := k.(starlark.String).GoString()
	r, found := nq[key]

	if !found {
		return nil, false, fmt.Errorf("no query named %q, queries: %v", key, maps.Keys(nq))
	}

	// Pure primitive query results
	return starUtils.Write(r), true, nil
}

var _ starlark.Mapping = (*QueryResults)(nil)

func (qr QueryResults) String() string {
	return fmt.Sprintf("QueryResults(%v)", maps.Keys(qr))
}
func (qr QueryResults) Type() string         { return "QueryResults" }
func (qr QueryResults) Freeze()              {}
func (qr QueryResults) Truth() starlark.Bool { return starlark.True }
func (qr QueryResults) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", qr.Type())
}

func (qr QueryResults) Get(k starlark.Value) (v starlark.Value, found bool, err error) {
	if k.Type() != "string" {
		return nil, false, fmt.Errorf("invalid key type, expected string")
	}
	key := k.(starlark.String).GoString()
	r, found := qr[key]

	if !found {
		keys := []string{}
		for k := range qr {
			keys = append(keys, k)
		}
		return nil, false, fmt.Errorf("no query result named %q, queries: %v", key, keys)
	}

	// Pure primitive query results
	return starUtils.Write(r), true, nil
}
