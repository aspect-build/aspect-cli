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

package flags

import (
	"fmt"
	"strings"
)

// MultiString is the golang implementation of bazel multi-string arguments that satisfies
// Value from cobra's Flags().Var functions.
type MultiString struct {
	value []string
}

// Set satisfies Value from cobra's Flags().Var functions.
func (s *MultiString) Set(value string) error {
	s.value = append(s.value, value)
	return nil
}

// Type satisfies Value from cobra's Flags().Var functions.
func (s *MultiString) Type() string {
	return "multiString"
}

// String satisfies Value from cobra's Flags().Var functions.
func (s *MultiString) String() string {
	return fmt.Sprintf("[ %s ]", strings.Join(s.value, ", "))
}

// First satisfies Value from cobra's Flags().Var functions
func (s *MultiString) First() string {
	return (s.value)[0]
}

func (s *MultiString) Get() []string {
	return s.value
}
