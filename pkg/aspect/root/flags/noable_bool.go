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
	"log"

	"github.com/spf13/pflag"
)

func RegisterNoableBool(flags *pflag.FlagSet, name string, value bool, usage string) *bool {
	result := value

	trueNB := noableBool{
		value:        &result,
		valueWhenSet: true,
	}
	flags.Var(&trueNB, name, usage)

	falseNB := noableBool{
		value:        &result,
		valueWhenSet: false,
	}
	flags.Var(&falseNB, "no"+name, usage)

	return &result
}

type noableBool struct {
	// The address of the actual value.
	value *bool
	// The value that should be set when the Set() function is called.
	valueWhenSet bool
}

func (nb *noableBool) Type() string {
	return "bool"
}

func (nb *noableBool) String() string {
	// Print the boolean representation of the value
	return fmt.Sprintf("%t", *nb.value)
}

func (nb *noableBool) Set(value string) error {
	// DEBUG BEGIN
	log.Printf("*** CHUCK:  value: %+#v", value)
	log.Printf("*** CHUCK:  nb.value: %+#v", nb.value)
	log.Printf("*** CHUCK:  nb.valueWhenSet: %+#v", nb.valueWhenSet)
	// DEBUG END
	*nb.value = nb.valueWhenSet
	return nil
}
