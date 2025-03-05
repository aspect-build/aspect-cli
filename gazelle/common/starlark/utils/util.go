package starlark

import (
	"fmt"
	"log"
	"reflect"

	"go.starlark.net/starlark"
)

var EmptyArgs = starlark.Tuple{}
var EmptyKwArgs = make([]starlark.Tuple, 0)
var EmptyStrings = make([]string, 0)
var EmptyStringBoolMap = make(map[string]bool)
var EmptyStringMap = make(map[string]string)

func Write(v interface{}) starlark.Value {
	if sv, isSV := v.(starlark.Value); isSV {
		return sv
	}

	// Primitive types
	switch v := v.(type) {
	case nil:
		return starlark.None
	case bool:
		return starlark.Bool(v)
	case string:
		return starlark.String(v)
	case int:
		return starlark.MakeInt(v)
	case int64:
		return starlark.MakeInt64(v)
	case float64:
		return starlark.Float(v)
	case []string:
		return WriteList(v, WriteString)
	case []interface{}:
		return WriteList(v, Write)
	case map[string]interface{}:
		return WriteMap(v, Write)
	}

	log.Panicf("Failed to write value %v of type %q", v, reflect.TypeOf(v))
	return nil
}

func WriteString(v string) starlark.Value {
	return starlark.String(v)
}

func WriteList[V any](a []V, f func(v V) starlark.Value) starlark.Value {
	l := make([]starlark.Value, 0, len(a))
	for _, v := range a {
		l = append(l, f(v))
	}
	return starlark.NewList(l)
}

func WriteStringList(a []string) starlark.Value {
	return WriteList(a, WriteString)
}

func WriteMap[K any](m map[string]K, f func(v K) starlark.Value) starlark.Value {
	d := starlark.NewDict(len(m))
	for k, v := range m {
		d.SetKey(starlark.String(k), f(v))
	}
	return d
}

func WriteStringMap(m map[string]string) starlark.Value {
	return WriteMap(m, WriteString)
}

func ReadBool(v starlark.Value) bool {
	return v.(starlark.Bool).Truth() == starlark.True
}
func ReadString(v starlark.Value) string {
	return v.(starlark.String).GoString()
}

func ReadList[V any](v starlark.Value, f func(v starlark.Value) V) []V {
	l := v.(*starlark.List)
	len := l.Len()
	a := make([]V, 0, len)
	for i := 0; i < len; i++ {
		a = append(a, f(l.Index(i)))
	}
	return a
}

func ReadTuple[V any](t starlark.Tuple, f func(v starlark.Value) V) []V {
	len := t.Len()
	a := make([]V, 0, len)
	for i := 0; i < len; i++ {
		a = append(a, f(t.Index(i)))
	}
	return a
}

func ReadStringList(l starlark.Value) []string {
	return ReadList(l, ReadString)
}

func ReadStringTuple(l starlark.Tuple) []string {
	return ReadTuple(l, ReadString)
}

func ForEachMapEntry(v starlark.Value, f func(k string, v starlark.Value)) {
	d := v.(*starlark.Dict)

	iter := d.Iterate()
	defer iter.Done()

	var k starlark.Value
	for iter.Next(&k) {
		v, _, _ := d.Get(k)
		f(ReadString(k), v)
	}
}

func Read(v starlark.Value) interface{} {
	return ReadRecurse(v, Read)
}

func ReadRecurse(v starlark.Value, read func(v starlark.Value) interface{}) interface{} {
	switch v := v.(type) {
	case starlark.NoneType:
		return nil
	case starlark.Bool:
		return v.Truth() == starlark.True
	case starlark.String:
		return v.GoString()
	case starlark.Int:
		i, _ := v.Int64()
		return i
	case starlark.Float:
		return float64(v)
	case *starlark.List:
		return ReadList(v, read)
	case *starlark.Dict:
		return ReadMap2(v, read)
	case starlark.Sequence:
		return readIterable(v, v.Len(), read)
	case starlark.Iterable:
		return readIterable(v, -1, read)
	case starlark.Indexable:
		return readIndexable(v, read)
	}

	log.Panicf("Failed to read starlark value %T", v)
	return nil
}

func readIterable(v starlark.Iterable, len int, read func(v starlark.Value) interface{}) []interface{} {
	iter := v.Iterate()
	defer iter.Done()

	a := make([]interface{}, 0, len)
	var x starlark.Value
	for iter.Next(&x) {
		a = append(a, read(x))
	}

	return a
}

func readIndexable(v starlark.Indexable, read func(v starlark.Value) interface{}) []interface{} {
	len := v.Len()
	a := make([]interface{}, 0, len)
	for i := 0; i < len; i++ {
		a = append(a, read(v.Index(i)))
	}
	return a
}

func ReadMap[K any](v starlark.Value, f func(k string, v starlark.Value) K) map[string]K {
	d := v.(*starlark.Dict)
	m := make(map[string]K, d.Len())

	iter := d.Iterate()
	defer iter.Done()

	var kv starlark.Value
	for iter.Next(&kv) {
		k := ReadString(kv)
		v, _, _ := d.Get(kv)
		m[k] = f(k, v)
	}

	return m
}

func ReadMap2[K any](v starlark.Value, f func(v starlark.Value) K) map[string]K {
	d := v.(*starlark.Dict)
	m := make(map[string]K, d.Len())

	iter := d.Iterate()
	defer iter.Done()

	var kv starlark.Value
	for iter.Next(&kv) {
		k := ReadString(kv)
		v, _, _ := d.Get(kv)
		m[k] = f(v)
	}

	return m
}

func ReadMapEntry[K any](v starlark.Value, key string, f func(v starlark.Value) K) K {
	m := v.(*starlark.Dict)
	val, exists, err := (*m).Get(starlark.String(key))

	if err != nil {
		log.Panicf("Failed to read map entry %q: %v", key, err)
	}

	if !exists {
		log.Panicf("Map entry %q does not exist in %v", key, v)
	}

	return f(val)
}

func ReadOptionalMapEntry[K any](v starlark.Value, key string, f func(v starlark.Value) K, defaultValue K) K {
	m := v.(*starlark.Dict)
	val, exists, err := (*m).Get(starlark.String(key))

	if err != nil {
		log.Panicf("Failed to read map entry '%s': %v", key, err)
	}

	if !exists {
		return defaultValue
	}

	return f(val)
}

func ReadMapStringEntry(m starlark.Value, key string) string {
	return ReadMapEntry(m, key, ReadString)
}

func ReadBoolMap(v starlark.Value) map[string]bool {
	return ReadMap2(v, ReadBool)
}

func ReadStringMap(v starlark.Value) map[string]string {
	return ReadMap2(v, ReadString)
}

// Looping: efficient utils for iterators, sequences etc

func MappedSequence[V any](a []V, f func(v V) starlark.Value) starlark.Sequence {
	return &mappedSequence[V]{values: a, mapper: f}
}

func MappedIterator[V any](a []V, f func(v V) starlark.Value) starlark.Iterator {
	return &mappedIterator[V]{values: a, mapper: f, i: 0}
}

type mappedSequence[V any] struct {
	values []V
	mapper func(v V) starlark.Value
}

var _ starlark.Sequence = (*mappedSequence[any])(nil)
var _ starlark.Indexable = (*mappedSequence[any])(nil)

func (s *mappedSequence[V]) String() string {
	return fmt.Sprintf("MappedSequence{%v}", s.values)
}
func (s *mappedSequence[V]) Type() string         { return "MappedSequence" }
func (s *mappedSequence[V]) Freeze()              {}
func (s *mappedSequence[V]) Truth() starlark.Bool { return starlark.True }
func (s *mappedSequence[V]) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", s.Type())
}
func (s *mappedSequence[V]) Iterate() starlark.Iterator {
	return MappedIterator(s.values, s.mapper)
}
func (s *mappedSequence[V]) Len() int {
	return len(s.values)
}
func (s *mappedSequence[V]) Index(i int) starlark.Value {
	return s.mapper(s.values[i])
}

type mappedIterator[V any] struct {
	values []V
	i      int
	mapper func(v V) starlark.Value
}

var _ starlark.Iterator = (*mappedIterator[any])(nil)

func (s *mappedIterator[V]) String() string {
	return fmt.Sprintf("MappedIterator{%v @ %v}", s.values, s.i)
}
func (s *mappedIterator[V]) Type() string         { return "MappedIterator" }
func (s *mappedIterator[V]) Freeze()              {}
func (s *mappedIterator[V]) Truth() starlark.Bool { return starlark.True }
func (s *mappedIterator[V]) Hash() (uint32, error) {
	return 0, fmt.Errorf("unhashable: %s", s.Type())
}
func (s *mappedIterator[V]) Next(p *starlark.Value) bool {
	if s.i < len(s.values) {
		*p = s.mapper(s.values[s.i])
		s.i = s.i + 1
		return true
	}
	return false
}
func (s mappedIterator[V]) Done() {}
