console.log(process.argv);
console.log('--@aspect-test/a--', require('@aspect-test/a').id());
console.log('--@aspect-test/b--', require('@aspect-test/b').id());
console.log('--@aspect-test/c--', require('@aspect-test/c').id());
console.log('--@aspect-test/h--', require('@aspect-test/h').id());
console.log('--@lib/b--', require('@lib/b').id());
console.log('--@lib/b_alias--', require('@lib/b_alias').id());
