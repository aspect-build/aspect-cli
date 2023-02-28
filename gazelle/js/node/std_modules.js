const modules = require('module')
    .builtinModules.filter((m) => !m.startsWith('_'))
    .sort();

console.log(
    `
// GENERATED FILE - DO NOT EDIT!

package gazelle

var NativeModules = []string{
	"${modules.join('",\n	"')}",
}
`.trim()
);
