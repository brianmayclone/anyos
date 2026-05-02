const leftPad = require('left-pad');
const isOdd = require('is-odd');

function describe(value) {
  return leftPad(String(value), 3, '0') + ':' + isOdd(value);
}

exports.describe = describe;

if (require.main === module) {
  const value = Number(process.argv[2] || 7);
  console.log(describe(value));
}
