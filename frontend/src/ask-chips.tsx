import { useState } from "react";
import type {
  Chip,
  GroupBy,
  Limit,
  Metric,
  Period,
  Query,
  SortBy,
  SortOrder,
  Weekday,
} from "./generated";
import { Select } from "./select";
import { Button, Icon } from "./ui";

const metrics: { value: Metric; label: string }[] = [
  { value: "plays", label: "Plays" },
  { value: "time_listened", label: "Time listened" },
  { value: "distinct_tracks", label: "Distinct tracks" },
  { value: "distinct_artists", label: "Distinct artists" },
  { value: "distinct_albums", label: "Distinct albums" },
  { value: "first_play", label: "First play" },
  { value: "last_play", label: "Last play" },
  { value: "streak", label: "Listening streak" },
  { value: "milestones", label: "Milestones" },
];

const periods: { value: Period; label: string }[] = [
  { value: "all_time", label: "All time" },
  { value: "today", label: "Today" },
  { value: "yesterday", label: "Yesterday" },
  { value: "this_week", label: "This week" },
  { value: "last_week", label: "Last week" },
  { value: "this_month", label: "This month" },
  { value: "last_month", label: "Last month" },
  { value: "this_year", label: "This year" },
  { value: "last_year", label: "Last year" },
];

const weekdays: { value: Weekday; label: string }[] = [
  { value: "monday", label: "Monday" },
  { value: "tuesday", label: "Tuesday" },
  { value: "wednesday", label: "Wednesday" },
  { value: "thursday", label: "Thursday" },
  { value: "friday", label: "Friday" },
  { value: "saturday", label: "Saturday" },
  { value: "sunday", label: "Sunday" },
];

const sorts: { value: SortBy; label: string }[] = [
  { value: "metric", label: "Metric" },
  { value: "date", label: "Date" },
];

const orders: { value: SortOrder; label: string }[] = [
  { value: "desc", label: "Descending" },
  { value: "asc", label: "Ascending" },
];

const limits: { value: Limit; label: string }[] = [
  { value: "one", label: "1" },
  { value: "five", label: "5" },
  { value: "ten", label: "10" },
  { value: "twenty_five", label: "25" },
  { value: "fifty", label: "50" },
  { value: "hundred", label: "100" },
];

const groups: { value: GroupBy; label: string }[] = [
  { value: "day", label: "Day" },
  { value: "week", label: "Week" },
  { value: "month", label: "Month" },
  { value: "year", label: "Year" },
  { value: "hour", label: "Hour" },
  { value: "weekday", label: "Weekday" },
  { value: "artist", label: "Artist" },
  { value: "album", label: "Album" },
  { value: "track", label: "Track" },
];

function currentValue(query: Query, key: string): string {
  switch (key) {
    case "metric":
      return query.metric;
    case "entity":
      return query.entity?.name ?? "";
    case "period":
      return query.period;
    case "year":
      return query.year === null ? "" : String(query.year);
    case "month":
      return query.month === null ? "" : String(query.month);
    case "weekday":
      return query.weekday ?? "";
    case "hour":
      return query.hour === null ? "" : String(query.hour);
    case "source":
      return query.source ?? "";
    case "group_by":
      return query.group_by ?? "";
    case "sort_by":
      return query.sort_by;
    case "order":
      return query.order;
    case "limit":
      return query.limit ?? "";
    default:
      return "";
  }
}

/** Apply one chip edit without inventing fields outside the DSL contract. */
export function editQuery(query: Query, key: string, value: string): Query {
  const next: Query = { ...query };
  switch (key) {
    case "metric":
      next.metric = value as Metric;
      break;
    case "entity":
      next.entity = {
        kind: query.entity?.kind ?? "artist",
        name: value,
        id: null,
      };
      break;
    case "period":
      next.period = value as Period;
      break;
    case "year":
      next.year = value === "" ? null : Number(value);
      break;
    case "month":
      next.month = value === "" ? null : Number(value);
      break;
    case "weekday":
      next.weekday = value === "" ? null : (value as Weekday);
      break;
    case "hour":
      next.hour = value === "" ? null : Number(value);
      break;
    case "source":
      next.source = value.trim() === "" ? null : value.trim();
      break;
    case "group_by":
      next.group_by = value === "" ? null : (value as GroupBy);
      break;
    case "sort_by":
      next.sort_by = value as SortBy;
      break;
    case "order":
      next.order = value as SortOrder;
      break;
    case "limit":
      next.limit = value === "" ? null : (value as Limit);
      break;
  }
  return next;
}

export function InterpretationChips({
  query,
  chips,
  onChange,
  disabled = false,
}: {
  query: Query;
  chips: Chip[];
  onChange: (query: Query) => void;
  disabled?: boolean;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  if (chips.length === 0) return null;
  return (
    <div className="ask-chips" aria-label="Interpreted question">
      {chips.map((chip) => (
        <div className="ask-chip-wrap" key={chip.key}>
          <button
            type="button"
            className={`ask-chip ${editing === chip.key ? "is-editing" : ""}`}
            aria-expanded={editing === chip.key}
            onClick={() => setEditing(editing === chip.key ? null : chip.key)}
            disabled={disabled}
          >
            <span className="ask-chip-label">{chip.label}</span>
            <span className="ask-chip-value">{chip.value}</span>
            <Icon>edit</Icon>
          </button>
          {editing === chip.key && (
            <ChipEditor
              chip={chip}
              value={currentValue(query, chip.key)}
              close={() => setEditing(null)}
              apply={(value) => {
                const next = editQuery(query, chip.key, value);
                setEditing(null);
                if (value !== currentValue(query, chip.key)) onChange(next);
              }}
            />
          )}
        </div>
      ))}
    </div>
  );
}

function ChipEditor({
  chip,
  value,
  apply,
  close,
}: {
  chip: Chip;
  value: string;
  apply: (value: string) => void;
  close: () => void;
}) {
  const options = (() => {
    switch (chip.key) {
      case "metric":
        return metrics;
      case "period":
        return periods;
      case "weekday":
        return weekdays;
      case "sort_by":
        return sorts;
      case "order":
        return orders;
      case "limit":
        return limits;
      case "group_by":
        return groups;
      default:
        return null;
    }
  })();
  return (
    <form
      className="ask-chip-editor"
      aria-label={`Edit ${chip.label}`}
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        apply(String(data.get("value") ?? ""));
      }}
    >
      {options ? (
        <Select
          value={value}
          onChange={apply}
          aria-label={`Edit ${chip.label}`}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </Select>
      ) : (
        <input
          name="value"
          defaultValue={value}
          inputMode={
            chip.key === "year" || chip.key === "month" || chip.key === "hour"
              ? "numeric"
              : "text"
          }
          aria-label={`Edit ${chip.label}`}
          autoFocus
        />
      )}
      <div className="ask-chip-editor-actions">
        {!options && (
          <Button type="submit" variant="primary">
            Apply
          </Button>
        )}
        <Button type="button" variant="ghost" onClick={close}>
          Close
        </Button>
      </div>
    </form>
  );
}
