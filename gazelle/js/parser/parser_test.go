package parser

import (
	"testing"
)

var testCases = []struct {
	desc, ts        string
	filename        string
	expectedImports []string
	expectedModules []string
}{
	{
		desc:     "empty",
		ts:       "",
		filename: "empty.ts",
	}, {
		desc: "import single quote",
		ts: `
			import dateFns from 'date-fns';
			// Make sure import is used. Esbuild ignores unused imports.
			const myDateFns = dateFns;
		`,
		filename:        "single.ts",
		expectedImports: []string{"date-fns"},
	}, {
		desc: "import double quote",
		ts: `
			import dateFns from "date-fns";
			// Make sure import is used. Esbuild ignores unused imports.
			const myDateFns = dateFns;
		`,
		filename:        "double.ts",
		expectedImports: []string{"date-fns"},
	}, {
		desc: "import two",
		ts: `
			import {format} from 'date-fns'
			import Puppy from '@/components/Puppy';

			export default function useMyImports() {
				format(new Puppy());
			}
		`,
		filename:        "two.ts",
		expectedImports: []string{"date-fns", "@/components/Puppy"},
	}, {
		desc: "import depth",
		ts: `
			import package from "from/internal/package";

			// Use the import.
			export default package;
		`,
		filename:        "depth.ts",
		expectedImports: []string{"from/internal/package"},
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
		filename:        "multiline.ts",
		expectedImports: []string{"date-fns", "~/constants"},
	},
	{
		desc:            "simple require",
		ts:              `const a = require("date-fns");`,
		filename:        "require.ts",
		expectedImports: []string{"date-fns"},
	},
	{
		desc:     "incorrect imports",
		ts:       `@import "~mapbox.js/dist/mapbox.css";`,
		filename: "actuallyScss.ts",
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
		filename:        "strings.ts",
		expectedImports: []string{},
	},
	{
		desc: "full import",
		ts: `
			import "mypolyfill";
			import "mypolyfill2";
		`,
		filename:        "full.ts",
		expectedImports: []string{"mypolyfill", "mypolyfill2"},
	},
	{
		desc:            "full require",
		ts:              `require("mypolyfill2");`,
		filename:        "fullRequire.ts",
		expectedImports: []string{"mypolyfill2"},
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
		filename:        "mixedImports.ts",
		expectedImports: []string{"vuex", "@vue/test-utils", "~/plugins/intersection-observer-polyfill", "~/plugins/intersect-directive", "./claims-section"},
	},
	{
		desc: "dynamic require",
		ts: `
			if (process.ENV.SHOULD_IMPORT) {
				// const old = require('oldmapbox.js');
				const leaflet = require('mapbox.js');
			}
		`,
		filename:        "dynamic.ts",
		expectedImports: []string{"mapbox.js"},
	},
	{
		desc: "regex require",
		ts: `
			var myRegexp = /import x from "y/;
		`,
		filename:        "regex.ts",
		expectedImports: []string{},
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
		filename:        "myComponent.tsx",
		expectedImports: []string{"react"},
	},
	{
		desc: "include unused imports",
		ts: `
			import "my/unused/package";
		`,
		filename:        "unusedImports.ts",
		expectedImports: []string{"my/unused/package"},
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
		filename:        "ExampleWithKeys.tsx",
		expectedImports: []string{"react", "react-i18next"},
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
		filename:        "sg-example-once-crashed.tsx",
		expectedImports: []string{"react"},
	},
	{
		desc: "ts type import",
		ts: `
			import type React from "react"
			import type { X } from "y"
		`,
		filename:        "types.ts",
		expectedImports: []string{"react", "y"},
	},
	{
		desc: "include imports only used as types",
		ts: `
			import { Foo } from "my/types";
			export const foo: Foo = 1
		`,
		filename:        "typeImport.ts",
		expectedImports: []string{"my/types"},
	},
	{
		desc: "include require()s only used as types",
		ts: `
			const { Foo } = require("my/types");
			export const foo: Foo = 1
		`,
		filename:        "typeRequire.ts",
		expectedImports: []string{"my/types"},
	},
	{
		desc: "include type-only imports",
		ts: `
			import type { Foo } from "my/types";
			export const foo: Foo = 1
		`,
		filename:        "typeImport.ts",
		expectedImports: []string{"my/types"},
	},
	{
		desc: "include unused type-only imports",
		ts: `
			import type { Foo } from "my/types";
		`,
		filename:        "typeImport-unused.ts",
		expectedImports: []string{"my/types"},
	},
	{
		desc: "module declaration",
		ts: `
			// https://www.typescriptlang.org/docs/handbook/modules.html#ambient-modules
			declare module 'module-x' {
				export var s: string;
			}

			// https://www.typescriptlang.org/docs/handbook/modules.html#shorthand-ambient-modules
			declare module 'module-with-no-body';

			declare /* 1 */ module /* 2 */ 'comments-2' /* 3 */;
			/* 1 */ declare module /* 2 */ 'comments-3'; /* 3 */
			declare module "module-quotes-1"
		`,
		filename:        "declare-module.ts",
		expectedModules: []string{"module-x", "module-with-no-body", "comments-2", "comments-3", "module-quotes-1"},
	},
	{
		desc: "declare module sub-imports",
		ts: `
			declare module 'lib-imports' {
				export * from 'lib-export-star';
				export * as foo from 'lib-export-star-as';
				import f /*c*/ from 'lib-from-default';

				import { x /*c*/  } /*c*/  from /*c*/ 'lib-impt'
				export { x } /*c*/
			}
		`,
		filename:        "declare-module-sub.ts",
		expectedModules: []string{"lib-imports"},
		expectedImports: []string{"lib-export-star", "lib-export-star-as", "lib-from-default", "lib-impt"},
	},
	{
		desc: "declare module protocol",
		ts: `
			declare module 'https://mod.com' {
				export * from 'ftp://ancient.com';

				export const a = 1
			}
		`,
		filename:        "declare-protocol-module.ts",
		expectedModules: []string{"https://mod.com"},
		expectedImports: []string{"ftp://ancient.com"},
	},
}

func RunParserTests(t *testing.T, parserPost string) {
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

func TestTreesitterParser(t *testing.T) {
	for _, tc := range testCases {
		t.Run(tc.desc, func(t *testing.T) {
			res, _ := ParseSource(tc.filename, []byte(tc.ts))

			if !equal(res.Imports, tc.expectedImports) {
				t.Errorf("Unexpected import results\nactual:  %#v;\nexpected: %#v\ntypescript code:\n%v", res.Imports, tc.expectedImports, tc.ts)
			}

			if !equal(res.Modules, tc.expectedModules) {
				t.Errorf("Unexpected module results\nactual:  %#v;\nexpected: %#v\ntypescript code:\n%v", res.Modules, tc.expectedModules, tc.ts)
			}
		})
	}
}
