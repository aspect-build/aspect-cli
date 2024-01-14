const packageJson = require('./package.json');

module.exports.id = `${packageJson.name}@${packageJson.version}`;
