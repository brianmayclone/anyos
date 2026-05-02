const _ = require('lodash');
const moment = require('moment');

function summarize() {
  const rows = [
    { group: 'core', value: 2 },
    { group: 'ui', value: 3 },
    { group: 'core', value: 5 }
  ];
  const grouped = _.groupBy(rows, 'group');
  const total = _.sumBy(grouped.core, 'value');
  const slug = _.camelCase('Any OS node runtime');
  const day = moment.utc('2026-05-02T10:30:00Z').format('YYYY-MM-DD');
  return total + ':' + slug + ':' + day;
}

exports.summarize = summarize;

if (require.main === module) {
  console.log(summarize());
}
