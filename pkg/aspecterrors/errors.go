/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package aspecterrors

const (
	// Bazel defines exit codes ~ 1-50: https://bazel.build/run/scripts#exit-codes
	// `bazel run` may propagate the exit code of the binary it runs.
	Failed                   = 1 // test/build/run build failure
	CLIError                 = 2 // bad cli args, bad env vars, etc.
	PartialOk                = 3 // build success, but: no tests found, query error, etc.
	NoTestsFound             = 4
	UnhandledOrInternalError = 37

	// Aspect CLI specific exit codes: 100 - ~200
	ConfigureFixed    = 110
	ConfigureDiff     = 111
	ConfigureNoConfig = 112
	LintFailure       = 113

	// Aspect Workflows specific exit codes: 200+
)

// ErrorList is a linked list for errors.
type ErrorList struct {
	head *errorNode
	tail *errorNode
	size int
}

// Insert inserts a new error into the linked list.
func (l *ErrorList) Insert(err error) {
	node := &errorNode{err: err}
	if l.head == nil {
		l.head = node
	} else {
		l.tail.next = node
	}
	l.tail = node
	l.size++
}

// Errors return a slice with all the elements in the linked list.
func (l *ErrorList) Errors() []error {
	errors := make([]error, 0, l.size)
	node := l.head
	for node != nil {
		errors = append(errors, node.err)
		node = node.next
	}
	return errors
}

type errorNode struct {
	next *errorNode
	err  error
}

// ExitError encapsulates an upstream error and an exit code. It is used by the
// aspect CLI main entrypoint to propagate meaningful exit error codes as the
// aspect CLI exit code.
type ExitError struct {
	Err      error
	ExitCode int
}

// Error returns the call to the encapsulated error.Error().
func (err *ExitError) Error() string {
	if err.Err != nil {
		return err.Err.Error()
	}
	return ""
}
