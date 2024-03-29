const packageJson = require('./package.json');
const e = require('@aspect-test/e');
const libB = require('@lib/b');
module.exports = {
    id: () =>
        `${packageJson.name}@${
            packageJson.version ? packageJson.version : '0.0.0'
        }`,
    idE: () => e.id(),
    idLibB: () => libB.id(),
};
