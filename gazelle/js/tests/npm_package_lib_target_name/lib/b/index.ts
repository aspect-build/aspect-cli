const packageJson = require('./package.json');
const { id: cStr } = require('@lib/c');

module.exports.id = `${packageJson.name}@${packageJson.version} + ${cStr}`;
