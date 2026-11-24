'use strict';

const WEEKDAY_EVENTS = {
  1: [['break-start', 12, 0], ['break-end', 13, 0]],
  2: [['break-start', 12, 0], ['break-end', 13, 0]],
  3: [['break-start', 12, 0], ['break-end', 13, 0]],
  4: [['break-start', 12, 0], ['break-end', 13, 0]],
  5: [['break-start', 11, 30], ['break-end', 13, 30]],
  6: [['break-start', 12, 0], ['break-end', 13, 0]],
  0: []
};

function dateKey(date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

function getScheduledWorkEvent(date = new Date()) {
  if (!(date instanceof Date) || Number.isNaN(date.getTime())) return null;
  const weekday = date.getDay();
  const candidates = [...(WEEKDAY_EVENTS[weekday] || []), ['workday-end', 17, 0]];
  const match = candidates.find(([, hour, minute]) => date.getHours() === hour && date.getMinutes() === minute);
  if (!match) return null;
  const [id, hour, minute] = match;
  return { id, key: `${dateKey(date)}:${id}`, hour, minute, weekday };
}

function workScheduleForDay(weekday) {
  if (!Number.isInteger(weekday) || weekday < 0 || weekday > 6) return [];
  const items = [...(WEEKDAY_EVENTS[weekday] || []), ['workday-end', 17, 0]];
  return items.map(([id, hour, minute]) => ({ id, hour, minute, weekday }));
}

module.exports = { getScheduledWorkEvent, workScheduleForDay };
