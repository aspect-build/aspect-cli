// Inherited ignore
import $ from 'jquery';
$('div').remove();

// Ignore from this BUILD in addition to the inherited
import foo from 'extra-ignore';
console.log(foo);

// Imported by other BUILD .js
export default 1;
