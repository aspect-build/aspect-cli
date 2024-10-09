package starlark

import (
	"fmt"
	"os"
	"strings"

	"go.starlark.net/starlark"
)

func ErrorStr(pre string, err error) string {
	if ee, isEvalError := err.(*starlark.EvalError); isEvalError {
		return evalErrorBacktrace(ee)
	}
	return err.Error()
}

// Modified version of starlark.EvalError.Backtrace(), starlark.CallStack.String() ...
// to filter out bazel sandbox, runfiles etc.
// See: go.starlark.net@v0.0.0-20240123142251-f86470692795/starlark/eval.go
func evalErrorBacktrace(e *starlark.EvalError) string {
	// If the topmost stack frame is a built-in function,
	// remove it from the stack and add print "Error in fn:".
	stack := e.CallStack
	suffix := ""
	if last := len(stack) - 1; last >= 0 && stack[last].Pos.Filename() == builtinFilename {
		suffix = " in " + stack[last].Name
		stack = stack[:last]
	}
	return fmt.Sprintf("Error%s: %s\n%s", suffix, e.Msg, evalCallbackString(stack))
}

func evalCallbackString(stack starlark.CallStack) string {
	strip := ""

	if pwd, pwdFound := os.LookupEnv("PWD"); pwdFound {
		strip = pwd
	}

	out := new(strings.Builder)
	if len(stack) > 0 {
		fmt.Fprintf(out, "Traceback (most recent call last):\n")
	}
	for _, fr := range stack {
		p := strings.TrimPrefix(fr.Pos.String(), strip)
		p = strings.Trim(p, " \n\r\t/\\")
		fmt.Fprintf(out, "  %s: in %s\n", p, fr.Name)
	}
	return out.String()
}

var builtinFilename = "<builtin>"
