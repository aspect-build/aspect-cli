package tests

import (
	"testing"

	"aspect.build/cli/gazelle/js/parser"
)

var testCases = []struct {
	desc, ts string
	filename string
	expected []string
	typeOnly bool
}{
	{
		desc:     "empty",
		ts:       "",
		filename: "empty.ts",
		expected: []string{},
	}, {
		desc: "import single quote",
		ts: `
			import dateFns from 'date-fns';
			// Make sure import is used. Esbuild ignores unused imports.
			const myDateFns = dateFns;
		`,
		filename: "single.ts",
		expected: []string{"date-fns"},
	}, {
		desc: "import double quote",
		ts: `
			import dateFns from "date-fns";
			// Make sure import is used. Esbuild ignores unused imports.
			const myDateFns = dateFns;
		`,
		filename: "double.ts",
		expected: []string{"date-fns"},
	}, {
		desc: "import two",
		ts: `
			import {format} from 'date-fns'
			import Puppy from '@/components/Puppy';

			export default function useMyImports() {
				format(new Puppy());
			}
		`,
		filename: "two.ts",
		expected: []string{"date-fns", "@/components/Puppy"},
	}, {
		desc: "import depth",
		ts: `
			import package from "from/internal/package";
			
			// Use the import.
			export default package;
		`,
		filename: "depth.ts",
		expected: []string{"from/internal/package"},
	}, {
		desc: "import multiline",
		ts: `
			import {format} from 'date-fns'
			import {
				CONST1,
				CONST2,
				CONST3,
			} from '~/constants';

			// Use the imports.
			format(CONST1, CONST2, CONST3);
		`,
		filename: "multiline.ts",
		expected: []string{"date-fns", "~/constants"},
	},
	{
		desc:     "simple require",
		ts:       `const a = require("date-fns");`,
		filename: "require.ts",
		expected: []string{"date-fns"},
	},
	{
		desc:     "incorrect imports",
		ts:       `@import "~mapbox.js/dist/mapbox.css";`,
		filename: "actuallyScss.ts",
		expected: []string{},
	},
	{
		desc: "ignores commented out imports",
		ts: `
			// takes ?inline out of the aliased import path, only if it's set
			// e.g. ~/path/to/file.svg?inline -> ~/path/to/file.svg
			'^~/(.+\\.svg)(\\?inline)?$': '<rootDir>$1',
			// const a = require("date-fns");
			// import {format} from 'date-fns';

			/*
			Also multi-line comments:
			const b = require("fs");
			import React from "react";
			*/
		`,
		filename: "comments.ts",
		expected: []string{},
	},
	{
		desc: "ignores imports inside of strings - both multi-line template strings and literal strings",
		ts: `
			const a = "import * as React from 'react';";
			const b = "var fs = require('fs');";
			const c = ` + "`" +
			`
			import * as React from 'react';
			const path = require('path');
						` + "`;",
		filename: "strings.ts",
		expected: []string{},
	},
	{
		desc: "full import",
		ts: `
			import "mypolyfill";
			import "mypolyfill2";
		`,
		filename: "full.ts",
		expected: []string{"mypolyfill", "mypolyfill2"},
	},
	{
		desc:     "full require",
		ts:       `require("mypolyfill2");`,
		filename: "fullRequire.ts",
		expected: []string{"mypolyfill2"},
	},
	{
		desc: "imports and full imports",
		ts: `
			import Vuex, { Store } from 'vuex';
			import { createLocalVue, shallowMount } from '@vue/test-utils';
			import '~/plugins/intersection-observer-polyfill';
			import '~/plugins/intersect-directive';
			import ClaimsSection from './claims-section';

			// Use the imports.
			export default { Store, shallowMount, ClaimsSection};
		`,
		filename: "mixedImports.ts",
		expected: []string{"vuex", "@vue/test-utils", "~/plugins/intersection-observer-polyfill", "~/plugins/intersect-directive", "./claims-section"},
	},
	{
		desc: "dynamic require",
		ts: `
			if (process.ENV.SHOULD_IMPORT) {
				// const old = require('oldmapbox.js');
				const leaflet = require('mapbox.js');
			}
		`,
		filename: "dynamic.ts",
		expected: []string{"mapbox.js"},
	},
	{
		desc: "regex require",
		ts: `
			var myRegexp = /import x from "y/;
		`,
		filename: "regex.ts",
		expected: []string{},
	},
	{
		desc: "tsx later in file",
		ts: `
			import React from "react";

			interface MyComponentProps {
			}
			const MyComponent : React.FC<MyComponentProps> = (props: MyComponentProps) => {
				return <div>hello</div>;
			}
		`,
		filename: "myComponent.tsx",
		expected: []string{"react"},
	},
	{
		desc: "include unused imports",
		ts: `
			import "my/unused/package";
		`,
		filename: "unusedImports.ts",
		expected: []string{"my/unused/package"},
	},
	{
		desc: "tsx later in file 2",
		ts: `
			import React from "react";
			import { Trans } from "react-i18next";

			const ExampleWithKeys = () => {
			return (
				<p>
				<Trans i18nKey="someKey" />
				</p>
			);
			};

			export default ExampleWithKeys;
		`,
		filename: "ExampleWithKeys.tsx",
		expected: []string{"react", "react-i18next"},
	},
	{
		desc: "tsx that once crashed with ts parser",
		ts: `
			import React from "react";
			export const a: React.FunctionComponent<React.PropsWithChildren<X>> = ({y}) => (
				<>
					{authProviders && (
						<ul className="list-group">
						</ul>
					)}
				</>
			})
		`,
		filename: "sg-example-once-crashed.tsx",
		expected: []string{"react"},
		typeOnly: true,
	},
	{
		desc: "ts type import",
		ts: `
			import type React from "react"
			import type { X } from "y"
		`,
		filename: "types.ts",
		expected: []string{"react", "y"},
		typeOnly: true,
	},
	{
		desc: "include imports only used as types",
		ts: `
			import { Foo } from "my/types";
			export const foo: Foo = 1
		`,
		filename: "typeImport.ts",
		expected: []string{"my/types"},
		typeOnly: true,
	},
	{
		desc: "include require()s only used as types",
		ts: `
			const { Foo } = require("my/types");
			export const foo: Foo = 1
		`,
		filename: "typeRequire.ts",
		expected: []string{"my/types"},
		typeOnly: true,
	},
	{
		desc: "include type-only imports",
		ts: `
			import type { Foo } from "my/types";
			export const foo: Foo = 1
		`,
		filename: "typeImport.ts",
		expected: []string{"my/types"},
		typeOnly: true,
	},
	{
		desc: "include unused type-only imports",
		ts: `
			import type { Foo } from "my/types";
		`,
		filename: "typeImport-unused.ts",
		expected: []string{"my/types"},
		typeOnly: true,
	},
}

func RunParserTests(t *testing.T, parser parser.Parser, includeTypeOnly bool, parserPost string) {
	for _, tc := range testCases {
		if !includeTypeOnly && tc.typeOnly {
			continue
		}

		t.Run(tc.desc+parserPost, func(t *testing.T) {
			actualImports, _ := parser.ParseImports(tc.filename, tc.ts)

			if !equal(actualImports, tc.expected) {
				t.Errorf("Inequality.\nactual:  %#v;\nexpected: %#v\ntypescript code:\n%v", actualImports, tc.expected, tc.ts)
			}
		})
	}
}

func equal[T comparable](a, b []T) bool {
	if len(a) != len(b) {
		return false
	}
	for i, v := range a {
		if v != b[i] {
			return false
		}
	}
	return true
}
