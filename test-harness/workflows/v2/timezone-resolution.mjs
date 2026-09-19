// Convert a wall-clock selection in an IANA zone without using the host zone.
// Ambiguous times return both instants; nonexistent times return no instant.
export function resolveLocalTime(local, timeZone) {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(local);
  if (!match) throw new RangeError('Enter a complete local date and time');
  const fields = match.slice(1).map(Number);
  const [year, month, day, hour, minute] = fields;
  const wall = new Date(0);
  wall.setUTCFullYear(year, month - 1, day);
  wall.setUTCHours(hour, minute, 0, 0);
  if (year < 2000 || year > 2100 || wall.getUTCFullYear() !== year ||
      wall.getUTCMonth() !== month - 1 || wall.getUTCDate() !== day ||
      wall.getUTCHours() !== hour || wall.getUTCMinutes() !== minute) {
    throw new RangeError('Invalid appointment date or time');
  }
  const formatter = new Intl.DateTimeFormat('en-US', {
    timeZone, calendar: 'iso8601', numberingSystem: 'latn', hourCycle: 'h23',
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit',
  });
  function parts(instant) {
    const values = Object.fromEntries(formatter.formatToParts(new Date(instant))
      .filter(part => part.type !== 'literal').map(part => [part.type, Number(part.value)]));
    return [values.year, values.month, values.day, values.hour, values.minute, values.second];
  }
  const offsets = new Set();
  // The controlled fixture supports UTC, New York and Vancouver in 2000–2100.
  // Sampling both sides of the date gathers both offsets at their DST changes.
  for (let hours = -48; hours <= 48; hours++) {
    const instant = wall.getTime() + hours * 3600000;
    const [y, m, d, h, min, sec] = parts(instant);
    offsets.add(Date.UTC(y, m - 1, d, h, min, sec) - instant);
  }
  return [...offsets].map(offset => wall.getTime() - offset)
    .filter(instant => parts(instant).every((value, index) => value === [...fields, 0][index]))
    .sort((left, right) => left - right)
    .map(instant => new Date(instant).toISOString());
}
