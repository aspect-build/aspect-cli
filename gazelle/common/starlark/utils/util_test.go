package starlark

import (
	"testing"

	"go.starlark.net/starlark"
)

func TestReadWrite(t *testing.T) {
	t.Run("nil <=> None", func(t *testing.T) {
		if Write(nil) != starlark.None {
			t.Errorf("Expected None")
		}

		if Read(starlark.None) != nil {
			t.Errorf("Expected nil")
		}
	})

	t.Run("bool <=> Bool", func(t *testing.T) {
		if Write(true) != starlark.Bool(true) {
			t.Errorf("Expected true")
		}

		if Read(starlark.Bool(true)) != true {
			t.Errorf("Expected true")
		}
	})

	t.Run("string <=> String", func(t *testing.T) {
		if Write("hello") != starlark.String("hello") {
			t.Errorf("Expected hello")
		}

		if Read(starlark.String("hello")) != "hello" {
			t.Errorf("Expected hello")
		}
	})

	t.Run("int <=> Int", func(t *testing.T) {
		if Write(123) != starlark.MakeInt(123) {
			t.Errorf("Expected 123")
		}

		if Read(starlark.MakeInt(123)) != int64(123) {
			t.Errorf("Expected 123")
		}
	})

	t.Run("float64 <=> Float", func(t *testing.T) {
		if Write(123.45) != starlark.Float(123.45) {
			t.Errorf("Expected 123.45")
		}

		if Read(starlark.Float(123.45)) != 123.45 {
			t.Errorf("Expected 123.45")
		}
	})

	t.Run("List => []interface{}", func(t *testing.T) {
		a := ([]interface{}{int64(1), "hello", true})
		l := Write(a).(*starlark.List)

		if len(a) != l.Len() {
			t.Errorf("Expected equal length")
		}

		l0, isInt := l.Index(0).(starlark.Int).Int64()
		if !isInt || a[0] != l0 {
			t.Errorf("Expected %v to be Int64", l0)
		}

		l1, isString := l.Index(1).(starlark.String)
		if !isString || a[1] != l1.GoString() {
			t.Errorf("Expected %v to be String", l1)
		}

		l2, isBool := l.Index(2).(starlark.Bool)
		if !isBool || a[2] != (l2.Truth() == starlark.True) {
			t.Errorf("Expected %v to be Bool", l2)
		}
	})

	t.Run("List <=> []interface{}", func(t *testing.T) {
		l := starlark.NewList([]starlark.Value{starlark.MakeInt(1), starlark.String("hello"), starlark.Bool(true)})
		a := Read(l).([]interface{})

		if len(a) != l.Len() {
			t.Errorf("Expected equal length")
		}

		l0, isInt := l.Index(0).(starlark.Int).Int64()
		if !isInt || a[0].(int64) != l0 {
			t.Errorf("Expected %v to be Int64", l0)
		}

		l1, isString := l.Index(1).(starlark.String)
		if !isString || a[1] != l1.GoString() {
			t.Errorf("Expected %v to be String", l1)
		}

		l2, isBool := l.Index(2).(starlark.Bool)
		if !isBool || a[2] != (l2.Truth() == starlark.True) {
			t.Errorf("Expected %v to be Bool", l2)
		}
	})
}
