/* Copyright 2016 The Bazel Authors. All rights reserved.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

   http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

// NOTE: synced from bazel-gazelle/cmd/gazelle/main.go

// Command gazelle is a BUILD file generator for Go projects.
// See "gazelle --help" for more details.
package gazelle

import (
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/language"
)

type command int

const (
	updateCmd command = iota
	fixCmd
)

var commandFromName = map[string]command{
	"fix":    fixCmd,
	"update": updateCmd,
	// NOTE: aspect-cli removed --help,  --update-repos
}

var nameFromCommand = []string{
	// keep in sync with definition above
	"update",
	"fix",
	// NOTE: aspect-cli removed --help,  --update-repos
}

func (cmd command) String() string {
	return nameFromCommand[cmd]
}

// NOTE: aspect-cli removed main()

// filterLanguages returns the subset of input languages that pass the config's
// filter, if any. Gazelle should not generate rules for languages not returned.
func filterLanguages(c *config.Config, langs []language.Language) []language.Language {
	if len(c.Langs) == 0 {
		return langs
	}

	var result []language.Language
	for _, inputLang := range langs {
		if containsLang(c.Langs, inputLang) {
			result = append(result, inputLang)
		}
	}
	return result
}

func containsLang(langNames []string, lang language.Language) bool {
	for _, langName := range langNames {
		if langName == lang.Name() {
			return true
		}
	}
	return false
}
