import {
  Children,
  isValidElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./ui";

/** A controlled, keyboard-operable field. Options remain ordinary JSX data. */
export function Select({
  value,
  onChange,
  children,
  disabled = false,
  searchable = false,
  id,
  "aria-label": label,
}: {
  value: string | number;
  onChange: (value: string) => void;
  children: ReactNode;
  disabled?: boolean;
  searchable?: boolean;
  id?: string;
  "aria-label": string;
}) {
  const uid = useId();
  const listId = `${uid}-options`;
  const trigger = useRef<HTMLButtonElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const search = useRef({ text: "", at: 0 });
  const values = Children.toArray(children).flatMap((child) => {
    if (
      !isValidElement<{
        value?: string | number;
        disabled?: boolean;
        children?: ReactNode;
      }>(child)
    )
      return [];
    const text = Children.toArray(child.props.children).join("");
    return [
      {
        value: String(child.props.value ?? text),
        label: text,
        disabled: !!child.props.disabled,
      },
    ];
  });
  const options = searchable
    ? values.filter((option) =>
        option.label
          .toLocaleLowerCase()
          .includes(String(value).toLocaleLowerCase()),
      )
    : values;
  const selected = options.findIndex(
    (option) => option.value === String(value),
  );
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [position, setPosition] = useState({
    left: 0,
    top: 0,
    width: 0,
    maxHeight: 320,
  });
  function reveal() {
    setActive(selected < 0 ? 0 : selected);
    setOpen(true);
  }
  function choose(index: number) {
    if (!options[index] || options[index].disabled) return;
    onChange(options[index].value);
    setOpen(false);
    (input.current || trigger.current)?.focus();
  }
  useLayoutEffect(() => {
    if (!open) return;
    function place() {
      const rect = (input.current || trigger.current)?.getBoundingClientRect();
      if (!rect) return;
      const below = innerHeight - rect.bottom - 12;
      const above = rect.top - 12;
      const down =
        below >= Math.min(240, options.length * 44 + 10) || below >= above;
      const height = Math.max(44, Math.min(320, down ? below : above));
      const width = Math.min(Math.max(rect.width, 160), innerWidth - 16);
      const actual = Math.min(height, options.length * 44 + 10);
      setPosition({
        left: Math.max(8, Math.min(rect.left, innerWidth - width - 8)),
        top: down ? rect.bottom + 6 : Math.max(8, rect.top - actual - 6),
        width,
        maxHeight: height,
      });
    }
    function outside(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        !trigger.current?.contains(event.target) &&
        !input.current?.contains(event.target) &&
        !popup.current?.contains(event.target)
      )
        setOpen(false);
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    document.addEventListener("pointerdown", outside);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      document.removeEventListener("pointerdown", outside);
    };
  }, [open, options.length]);
  useEffect(() => {
    if (open)
      document
        .getElementById(`${uid}-${active}`)
        ?.scrollIntoView({ block: "nearest" });
  }, [active, open, uid]);
  function keyboard(event: KeyboardEvent<HTMLElement>) {
    const { key } = event;
    if (key === "Tab") {
      setOpen(false);
      return;
    }
    if (key === "Escape") {
      if (!open) return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      return;
    }
    if (
      [
        "ArrowDown",
        "ArrowUp",
        ...(!searchable ? ["Home", "End"] : []),
      ].includes(key)
    ) {
      event.preventDefault();
      if (!open) {
        reveal();
        return;
      }
      const enabled = options
        .map((o, i) => (o.disabled ? -1 : i))
        .filter((i) => i >= 0);
      if (!enabled.length) return;
      const current = enabled.indexOf(active);
      setActive(
        key === "Home"
          ? enabled[0]
          : key === "End"
            ? enabled[enabled.length - 1]
            : enabled[
                (current + (key === "ArrowDown" ? 1 : -1) + enabled.length) %
                  enabled.length
              ],
      );
    } else if (key === "Enter" || (!searchable && key === " ")) {
      event.preventDefault();
      if (open) choose(active);
      else reveal();
    } else if (
      !searchable &&
      key.length === 1 &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      event.preventDefault();
      const now = Date.now();
      const text =
        (now - search.current.at < 700 ? search.current.text : "") +
        key.toLocaleLowerCase();
      search.current = { text, at: now };
      const index = options.findIndex(
        (o) => !o.disabled && o.label.toLocaleLowerCase().startsWith(text),
      );
      if (index >= 0) {
        setActive(index);
        setOpen(true);
      }
    }
  }
  return (
    <span className="select-control">
      {searchable ? (
        <>
          <input
            ref={input}
            id={id}
            className="select-trigger select-search"
            role="combobox"
            aria-label={label}
            aria-autocomplete="list"
            aria-expanded={open}
            aria-controls={open ? listId : undefined}
            aria-activedescendant={
              open && options.length ? `${uid}-${active}` : undefined
            }
            value={value}
            required
            disabled={disabled}
            onFocus={() => {
              if (selected < 0 && options.length) reveal();
            }}
            onKeyDown={keyboard}
            onBlur={() => setOpen(false)}
            onChange={(event) => {
              const next = event.target.value;
              onChange(next);
              setActive(0);
              setOpen(
                !values.some((option) => option.value === next) &&
                  values.some((option) =>
                    option.label
                      .toLocaleLowerCase()
                      .includes(next.toLocaleLowerCase()),
                  ),
              );
            }}
          />
          <span className="select-search-icon">
            <Icon>expand_more</Icon>
          </span>
        </>
      ) : (
        <button
          ref={trigger}
          id={id}
          className="select-trigger"
          type="button"
          role="combobox"
          aria-label={label}
          aria-expanded={open}
          aria-haspopup="listbox"
          aria-controls={open ? listId : undefined}
          aria-activedescendant={open ? `${uid}-${active}` : undefined}
          data-value={String(value)}
          disabled={disabled}
          onKeyDown={keyboard}
          onBlur={() => setOpen(false)}
          onClick={() => (open ? setOpen(false) : reveal())}
        >
          <span>{options[selected]?.label || "Select…"}</span>
          <Icon>expand_more</Icon>
        </button>
      )}
      {open &&
        createPortal(
          <div
            ref={popup}
            id={listId}
            className="select-options"
            role="listbox"
            aria-label={`${label} options`}
            style={position}
            onMouseDown={(e) => e.preventDefault()}
          >
            {options.map((option, index) => (
              <div
                key={option.value}
                id={`${uid}-${index}`}
                role="option"
                aria-selected={index === selected}
                aria-disabled={option.disabled || undefined}
                data-value={option.value}
                className={index === active ? "active" : ""}
                onPointerMove={() => !option.disabled && setActive(index)}
                onClick={() => choose(index)}
              >
                <span>{option.label}</span>
                {index === selected && <Icon>check</Icon>}
              </div>
            ))}
            {!options.length && (
              <p className="select-empty" role="status">
                No matching options
              </p>
            )}
          </div>,
          document.body,
        )}
    </span>
  );
}
