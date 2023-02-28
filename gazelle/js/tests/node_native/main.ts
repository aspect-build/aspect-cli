const fs = require('fs');
const { exists } = require('node:fs');

console.log(fs.exists('foo'), exists('bar'));
