const packageJson = require('./package.json');
const { id: bStr } = require('@lib/b');

module.exports.id = `${packageJson.name}@${packageJson.version} + ${bStr}`;
