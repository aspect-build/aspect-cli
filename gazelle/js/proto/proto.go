package gazelle

import (
	"bytes"
	"io/ioutil"
	"log"
	"path"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"github.com/bazelbuild/bazel-gazelle/language"
	proto_config "github.com/bazelbuild/bazel-gazelle/language/proto"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

var protoRe = buildProtoRegexp()

func GetProtoLibraries(args language.GenerateArgs, result *language.GenerateResult) []*rule.Rule {
	var rules []*rule.Rule

	pc := proto_config.GetProtoConfig(args.Config)
	if pc != nil && pc.Mode.ShouldGenerateRules() {
		// proto_library rules managed by the proto language gazelle plugin
		rules = args.OtherGen
	} else {
		// Existing rules, maybe added manually and not generated
		if args.File != nil {
			rules = args.File.Rules
		}
	}

	protos := make([]*rule.Rule, 0, len(rules))

	for _, r := range rules {
		if r.Kind() == "proto_library" {
			protos = append(protos, r)
		}
	}

	return protos
}

func ToTsImports(src string) []string {
	src = src[:len(src)-len(path.Ext(src))]
	return []string{
		src + "_connect", // TODO: only add when a service is defined
		src + "_pb",
	}
}

func ToTsPaths(src string) []string {
	src = src[:len(src)-len(path.Ext(src))]
	return []string{
		src + "_connect.d.ts", // TODO: only add when a service is defined
		src + "_pb.d.ts",
	}
}

func GetProtoImports(filepath string) ([]string, error) {
	content, err := ioutil.ReadFile(filepath)
	if err != nil {
		return nil, err
	}

	imports := make([]string, 0)

	for _, match := range protoRe.FindAllSubmatch(content, -1) {
		switch {
		case match[importSubexpIndex] != nil:
			imp := unquoteProtoString(match[importSubexpIndex])
			imports = append(imports, imp)

		default:
			// Comment matched. Nothing to extract.
		}
	}

	sort.Strings(imports)

	return imports, nil
}

// Based on:
//
//	https://protobuf.dev/reference/protobuf/proto3-spec/#import_statement
//	https://github.com/bazelbuild/bazel-gazelle/blob/v0.32.0/language/proto/fileinfo.go#L106
func buildProtoRegexp() *regexp.Regexp {
	hexEscape := `\\[xX][0-9a-fA-f]{2}`
	octEscape := `\\[0-7]{3}`
	charEscape := `\\[abfnrtv'"\\]`
	charValue := strings.Join([]string{hexEscape, octEscape, charEscape, "[^\x00\\'\\\"\\\\]"}, "|")
	strLit := `'(?:` + charValue + `|")*'|"(?:` + charValue + `|')*"`
	importStmt := `\bimport\s*(?:public|weak)?\s*(?P<import>` + strLit + `)\s*;`
	return regexp.MustCompile(importStmt)
}

const importSubexpIndex = 1

// Copy of https://github.com/bazelbuild/bazel-gazelle/blob/v0.32.0/language/proto/fileinfo.go#L115-L138
func unquoteProtoString(q []byte) string {
	// Adjust quotes so that Unquote is happy. We need a double quoted string
	// without unescaped double quote characters inside.
	noQuotes := bytes.Split(q[1:len(q)-1], []byte{'"'})
	if len(noQuotes) != 1 {
		for i := 0; i < len(noQuotes)-1; i++ {
			if len(noQuotes[i]) == 0 || noQuotes[i][len(noQuotes[i])-1] != '\\' {
				noQuotes[i] = append(noQuotes[i], '\\')
			}
		}
		q = append([]byte{'"'}, bytes.Join(noQuotes, []byte{'"'})...)
		q = append(q, '"')
	}
	if q[0] == '\'' {
		q[0] = '"'
		q[len(q)-1] = '"'
	}

	s, err := strconv.Unquote(string(q))
	if err != nil {
		log.Panicf("unquoting string literal %s from proto: %v", q, err)
	}
	return s
}
