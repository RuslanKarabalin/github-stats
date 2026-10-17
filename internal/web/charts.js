(() => {
  "use strict";

  const source = document.getElementById("chart-data");
  if (!source) return;
  const data = JSON.parse(source.textContent);

  const NS = "http://www.w3.org/2000/svg";
  const DAY = 86400000;
  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const fmt = new Intl.NumberFormat("en-US");
  const fmt1 = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });
  const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });

  // ---------- helpers ----------

  function svgEl(tag, attrs, parent) {
    const node = document.createElementNS(NS, tag);
    for (const [key, value] of Object.entries(attrs || {})) node.setAttribute(key, value);
    if (parent) parent.appendChild(node);
    return node;
  }

  function htmlEl(tag, className, parent, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    if (parent) parent.appendChild(node);
    return node;
  }

  function parseDate(s) {
    const [y, m, d] = s.split("-").map(Number);
    return new Date(Date.UTC(y, m - 1, d));
  }

  function isoDate(date) {
    return date.toISOString().slice(0, 10);
  }

  function dayLabel(date) {
    return date.toLocaleDateString("en-US", {
      weekday: "short", month: "short", day: "numeric", year: "numeric", timeZone: "UTC",
    });
  }

  function monthLabel(key) {
    const [y, m] = key.split("-").map(Number);
    return `${MONTHS[m - 1]} ${y}`;
  }

  function plural(n, word) {
    return n === 1 ? word : word + "s";
  }

  function niceScale(maxValue, ticks) {
    if (!(maxValue > 0)) return { max: 1, step: 1 };
    const raw = maxValue / ticks;
    const magnitude = Math.pow(10, Math.floor(Math.log10(raw)));
    const norm = raw / magnitude;
    const factor = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 2.5 ? 2.5 : norm <= 5 ? 5 : 10;
    const step = factor * magnitude;
    return { max: Math.ceil(maxValue / step) * step, step };
  }

  // Redraws on width change so charts stay crisp at any size.
  function responsive(container, draw) {
    let width = 0;
    const redraw = () => {
      const next = Math.floor(container.clientWidth);
      if (next > 0 && next !== width) {
        width = next;
        container.replaceChildren();
        draw(width);
      }
    };
    // Drawing on the next frame keeps the observer from re-triggering itself.
    let frame = 0;
    new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(redraw);
    }).observe(container);
    return () => {
      width = 0;
      redraw();
    };
  }

  // Path for a column with a 4px rounded data-end and a square baseline.
  function columnPath(x, y, w, h, r) {
    r = Math.min(r, w / 2, h);
    return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
  }

  // ---------- tooltip ----------

  const tip = htmlEl("div", "tooltip", document.body);
  tip.setAttribute("role", "status");
  tip.hidden = true;

  // rows: [{ value, label, slot }]; slot draws a short line key in that series color.
  function showTip(x, y, title, rows) {
    tip.replaceChildren();
    htmlEl("div", "tip-title", tip, title);
    for (const row of rows) {
      const line = htmlEl("div", "tip-row", tip);
      if (row.slot !== undefined) htmlEl("span", `tip-key s${row.slot}`, line);
      htmlEl("strong", "", line, row.value);
      if (row.label) htmlEl("span", "tip-label", line, row.label);
    }
    tip.hidden = false;

    const pad = 14;
    const rect = tip.getBoundingClientRect();
    let left = x + pad;
    let top = y + pad;
    if (left + rect.width > window.innerWidth - 8) left = x - rect.width - pad;
    if (top + rect.height > window.innerHeight - 8) top = y - rect.height - pad;
    tip.style.left = `${Math.max(8, left)}px`;
    tip.style.top = `${Math.max(8, top)}px`;
  }

  function hideTip() {
    tip.hidden = true;
  }

  function tipAnchor(node) {
    const rect = node.getBoundingClientRect();
    return [rect.left + rect.width / 2, rect.top + rect.height / 2];
  }

  // Hover and keyboard focus show the same tooltip.
  function bindTip(node, content) {
    node.addEventListener("pointermove", (e) => {
      const c = content();
      showTip(e.clientX, e.clientY, c.title, c.rows);
    });
    node.addEventListener("pointerleave", hideTip);
    node.addEventListener("focus", () => {
      const c = content();
      showTip(...tipAnchor(node), c.title, c.rows);
    });
    node.addEventListener("blur", hideTip);
  }

  window.addEventListener("scroll", hideTip, { passive: true });

  for (const node of document.querySelectorAll("[data-tip]")) {
    bindTip(node, () => ({
      title: node.dataset.tip,
      rows: [{ value: node.dataset.tipValue, label: node.dataset.tipLabel }],
    }));
  }

  // ---------- data table twin ----------

  function dataTable(holder, headers, rows) {
    holder.replaceChildren();
    const details = htmlEl("details", "data-view", holder);
    htmlEl("summary", "", details, "Show data table");
    const scroll = htmlEl("div", "data-scroll", details);
    const table = htmlEl("table", "", scroll);
    const head = htmlEl("tr", "", htmlEl("thead", "", table));
    headers.forEach((h, i) => htmlEl("th", i > 0 ? "num" : "", head, h));
    const body = htmlEl("tbody", "", table);
    for (const row of rows) {
      const tr = htmlEl("tr", "", body);
      row.forEach((cell, i) => htmlEl("td", i > 0 ? "num" : "", tr, cell));
    }
  }

  function legend(holder, series) {
    const list = htmlEl("ul", "legend chart-legend", holder);
    for (const s of series) {
      const item = htmlEl("li", "", list);
      htmlEl("span", `swatch s${s.slot}`, item);
      htmlEl("span", "name", item, s.name);
    }
  }

  // ---------- y grid shared by area and column charts ----------

  function drawGrid(svg, m, plotWidth, plotHeight, scale, format) {
    const grid = svgEl("g", { class: "grid" }, svg);
    for (let v = 0; v <= scale.max + scale.step / 2; v += scale.step) {
      const y = m.top + plotHeight - (v / scale.max) * plotHeight;
      svgEl("line", {
        x1: m.left, x2: m.left + plotWidth, y1: y, y2: y,
        class: v === 0 ? "axis" : "gridline",
      }, grid);
      const label = svgEl("text", { x: m.left - 8, y: y + 4, class: "tick", "text-anchor": "end" }, grid);
      label.textContent = format(v);
    }
  }

  // ---------- contribution heatmap ----------

  function heatmap(root) {
    const counts = new Map(data.days.map((d) => [d.date, d.count]));
    const today = parseDate(data.today);
    const years = (data.years || []).map((y) => y.year).sort((a, b) => b - a);

    const options = [{ key: "last", label: "Last 12 months" }].concat(
      years.map((y) => ({ key: String(y), label: String(y) })),
    );
    let selected = "last";

    const controls = root.querySelector(".segmented");
    const chart = root.querySelector(".chart");
    const total = root.querySelector(".heatmap-total");
    const tableSlot = root.querySelector(".table-slot");

    const buttons = options.map((opt) => {
      const button = htmlEl("button", "", controls, opt.label);
      button.type = "button";
      button.addEventListener("click", () => {
        selected = opt.key;
        buttons.forEach((b, i) => b.setAttribute("aria-pressed", String(options[i].key === selected)));
        redraw();
      });
      button.setAttribute("aria-pressed", String(opt.key === selected));
      return button;
    });

    function range() {
      if (selected === "last") {
        const start = new Date(today);
        start.setUTCFullYear(start.getUTCFullYear() - 1);
        return { start: new Date(start.getTime() + DAY), end: today };
      }
      const year = Number(selected);
      const end = new Date(Date.UTC(year, 11, 31));
      return { start: new Date(Date.UTC(year, 0, 1)), end: end > today ? today : end };
    }

    function draw(width) {
      const { start, end } = range();
      const gridStart = new Date(start.getTime() - start.getUTCDay() * DAY);
      const weeks = Math.floor((end - gridStart) / DAY / 7) + 1;

      const labelWidth = 30;
      const top = 18;
      const step = Math.max(11, Math.min(17, Math.floor((width - labelWidth) / weeks)));
      const size = step - 3;

      // Quartiles of the active days in view, like GitHub's own calendar.
      const values = [];
      let sum = 0;
      const months = new Map();
      for (let t = start.getTime(); t <= end.getTime(); t += DAY) {
        const date = new Date(t);
        const c = counts.get(isoDate(date)) || 0;
        if (c > 0) values.push(c);
        sum += c;
        const key = isoDate(date).slice(0, 7);
        months.set(key, (months.get(key) || 0) + c);
      }
      values.sort((a, b) => a - b);
      const q = (p) => values[Math.min(values.length - 1, Math.floor(p * values.length))] || 0;
      const thresholds = [q(0.25), q(0.5), q(0.75)];
      const level = (c) => (c === 0 ? 0 : c <= thresholds[0] ? 1 : c <= thresholds[1] ? 2 : c <= thresholds[2] ? 3 : 4);

      const svg = svgEl("svg", {
        width: labelWidth + weeks * step,
        height: top + 7 * step,
        role: "img",
        "aria-label": `Contribution calendar: ${fmt.format(sum)} contributions`,
      }, chart);

      ["Mon", "Wed", "Fri"].forEach((name, i) => {
        const text = svgEl("text", { x: 0, y: top + (i * 2 + 1) * step + size - 1, class: "tick" }, svg);
        text.textContent = name;
      });

      let lastLabelCol = -4;
      for (let col = 0; col < weeks; col++) {
        for (let row = 0; row < 7; row++) {
          const date = new Date(gridStart.getTime() + (col * 7 + row) * DAY);
          if (date < start || date > end) continue;

          if (date.getUTCDate() === 1 || (col === 0 && row === start.getUTCDay())) {
            if (col - lastLabelCol >= 3) {
              const label = svgEl("text", { x: labelWidth + col * step, y: 11, class: "tick" }, svg);
              label.textContent = MONTHS[date.getUTCMonth()];
              lastLabelCol = col;
            }
          }

          const c = counts.get(isoDate(date)) || 0;
          svgEl("rect", {
            x: labelWidth + col * step,
            y: top + row * step,
            width: size,
            height: size,
            rx: 2,
            class: `cell h${level(c)}`,
            "data-date": isoDate(date),
            "data-count": c,
          }, svg);
        }
      }

      svg.addEventListener("pointermove", (e) => {
        const cell = e.target.closest(".cell");
        if (!cell) return hideTip();
        const c = Number(cell.dataset.count);
        showTip(e.clientX, e.clientY, dayLabel(parseDate(cell.dataset.date)), [
          { value: fmt.format(c), label: plural(c, "contribution") },
        ]);
      });
      svg.addEventListener("pointerleave", hideTip);

      total.textContent = `${fmt.format(sum)} ${plural(sum, "contribution")} in ${
        selected === "last" ? "the last 12 months" : selected
      }`;

      dataTable(tableSlot, ["Month", "Contributions"],
        [...months].map(([key, c]) => [monthLabel(key), fmt.format(c)]));
    }

    const redraw = responsive(chart, draw);
  }

  // ---------- monthly area chart ----------

  function areaChart(root, points) {
    const chart = root.querySelector(".chart");
    const n = points.length;
    const peak = points.reduce((best, p, i) => (p.count > points[best].count ? i : best), 0);

    function draw(width) {
      const height = 240;
      const m = { top: 24, right: 16, bottom: 28, left: 44 };
      const pw = width - m.left - m.right;
      const ph = height - m.top - m.bottom;
      const scale = niceScale(points[peak].count, 4);
      const x = (i) => m.left + (n === 1 ? pw / 2 : (i * pw) / (n - 1));
      const y = (v) => m.top + ph - (v / scale.max) * ph;

      const svg = svgEl("svg", {
        width, height, tabindex: 0, role: "img",
        "aria-label": "Contributions per month. Use the arrow keys to move between months.",
      }, chart);

      drawGrid(svg, m, pw, ph, scale, (v) => compact.format(v));

      // X labels: Januaries for long ranges, every few months for short ones.
      const minGap = 56;
      let candidates = points.map((_, i) => i);
      if (n > 24) candidates = candidates.filter((i) => points[i].month.endsWith("-01"));
      const perLabel = n > 1 ? pw / (n - 1) : pw;
      const spacing = n > 24 ? perLabel * 12 : perLabel;
      const stride = Math.max(1, Math.ceil(minGap / spacing));
      candidates.forEach((i, k) => {
        if (k % stride !== 0) return;
        const [yy, mm] = points[i].month.split("-");
        const text = svgEl("text", { x: x(i), y: height - 8, class: "tick", "text-anchor": "middle" }, svg);
        text.textContent = n > 24 ? yy : `${MONTHS[Number(mm) - 1]} ${yy.slice(2)}`;
      });

      const linePoints = points.map((p, i) => `${x(i)},${y(p.count)}`).join("L");
      svgEl("path", { d: `M${x(0)},${y(0)}L${linePoints}L${x(n - 1)},${y(0)}Z`, class: "area" }, svg);
      svgEl("path", { d: `M${linePoints}`, class: "line" }, svg);

      // Direct label on the extreme only.
      if (points[peak].count > 0) {
        svgEl("circle", { cx: x(peak), cy: y(points[peak].count), r: 4, class: "dot" }, svg);
        const label = svgEl("text", {
          x: Math.min(Math.max(x(peak), m.left + 40), m.left + pw - 40),
          y: y(points[peak].count) - 10,
          class: "value-label",
          "text-anchor": "middle",
        }, svg);
        label.textContent = `Peak: ${fmt.format(points[peak].count)} in ${monthLabel(points[peak].month)}`;
      }

      const hair = svgEl("line", { y1: m.top, y2: m.top + ph, class: "crosshair", visibility: "hidden" }, svg);
      const marker = svgEl("circle", { r: 4, class: "dot", visibility: "hidden" }, svg);
      const overlay = svgEl("rect", { x: m.left, y: m.top, width: pw, height: ph, fill: "transparent" }, svg);

      let current = n - 1;
      function show(i, clientX, clientY) {
        current = i;
        hair.setAttribute("x1", x(i));
        hair.setAttribute("x2", x(i));
        marker.setAttribute("cx", x(i));
        marker.setAttribute("cy", y(points[i].count));
        hair.setAttribute("visibility", "visible");
        marker.setAttribute("visibility", "visible");
        if (clientX === undefined) [clientX, clientY] = tipAnchor(marker);
        const partial = points[i].month === data.today.slice(0, 7) ? " (so far)" : "";
        showTip(clientX, clientY, monthLabel(points[i].month) + partial, [
          { value: fmt.format(points[i].count), label: plural(points[i].count, "contribution"), slot: 1 },
        ]);
      }
      function hide() {
        hair.setAttribute("visibility", "hidden");
        marker.setAttribute("visibility", "hidden");
        hideTip();
      }

      overlay.addEventListener("pointermove", (e) => {
        const box = svg.getBoundingClientRect();
        const px = e.clientX - box.left;
        const i = n === 1 ? 0 : Math.round(((px - m.left) / pw) * (n - 1));
        show(Math.max(0, Math.min(n - 1, i)), e.clientX, e.clientY);
      });
      overlay.addEventListener("pointerleave", hide);
      svg.addEventListener("focus", () => show(current));
      svg.addEventListener("blur", hide);
      svg.addEventListener("keydown", (e) => {
        const delta = { ArrowLeft: -1, ArrowRight: 1, Home: -n, End: n }[e.key];
        if (delta === undefined) return;
        e.preventDefault();
        show(Math.max(0, Math.min(n - 1, current + delta)));
      });
    }

    responsive(chart, draw);

    dataTable(root.querySelector(".table-slot"), ["Month", "Contributions"],
      points.slice().reverse().map((p) => [monthLabel(p.month), fmt.format(p.count)]));
  }

  // ---------- column chart (single or stacked) ----------

  // opts: { categories, series: [{ key, name, slot }], value(i, key), format, unit }
  function columnChart(root, opts) {
    const chart = root.querySelector(".chart");
    const n = opts.categories.length;
    const totals = opts.categories.map((_, i) => opts.series.reduce((sum, s) => sum + opts.value(i, s.key), 0));
    const multi = opts.series.length > 1;

    if (multi) legend(root.querySelector(".legend-slot"), opts.series);

    function content(i) {
      const rows = opts.series.map((s) => ({
        value: opts.format(opts.value(i, s.key)),
        label: s.name,
        slot: s.slot,
      }));
      if (multi) rows.push({ value: opts.format(totals[i]), label: "total" });
      else rows[0].label = opts.unit;
      return { title: opts.categories[i], rows };
    }

    function draw(width) {
      const height = 240;
      const m = { top: 22, right: 8, bottom: 28, left: 44 };
      const pw = width - m.left - m.right;
      const ph = height - m.top - m.bottom;
      const scale = niceScale(Math.max(...totals), 4);
      const band = pw / n;
      const barWidth = Math.min(24, band * 0.6);
      const y = (v) => (v / scale.max) * ph;

      const svg = svgEl("svg", { width, height }, chart);
      drawGrid(svg, m, pw, ph, scale, (v) => compact.format(v));

      const labelStride = Math.max(1, Math.ceil(40 / band));
      const baseline = m.top + ph;

      opts.categories.forEach((category, i) => {
        const cx = m.left + band * i + band / 2;
        const group = svgEl("g", { class: "column" }, svg);

        let top = baseline;
        const drawn = opts.series.filter((s) => opts.value(i, s.key) > 0);
        drawn.forEach((s, k) => {
          const h = y(opts.value(i, s.key));
          // 2px surface gap between stacked segments.
          const gap = k > 0 ? 2 : 0;
          const segHeight = h - gap;
          const segTop = top - h;
          if (segHeight > 0) {
            const isTop = k === drawn.length - 1;
            const shape = isTop
              ? svgEl("path", { d: columnPath(cx - barWidth / 2, segTop, barWidth, segHeight, 4) }, group)
              : svgEl("rect", { x: cx - barWidth / 2, y: segTop, width: barWidth, height: segHeight }, group);
            shape.setAttribute("class", `s${s.slot}`);
          }
          top = segTop;
        });

        if (totals[i] > 0 && band >= 26) {
          const cap = svgEl("text", { x: cx, y: top - 6, class: "value-label", "text-anchor": "middle" }, svg);
          cap.textContent = opts.capFormat ? opts.capFormat(totals[i]) : compact.format(totals[i]);
        }

        if (i % labelStride === 0) {
          const label = svgEl("text", { x: cx, y: height - 8, class: "tick", "text-anchor": "middle" }, svg);
          label.textContent = category;
        }

        // The whole band is the hit target, larger than the mark.
        const hit = svgEl("rect", {
          x: m.left + band * i, y: m.top, width: band, height: ph,
          fill: "transparent", tabindex: 0, role: "img",
          "aria-label": `${category}: ${opts.format(totals[i])} ${opts.unit}`,
        }, svg);
        bindTip(hit, () => content(i));
        hit.addEventListener("pointerenter", () => group.classList.add("hover"));
        hit.addEventListener("pointerleave", () => group.classList.remove("hover"));
        hit.addEventListener("focus", () => group.classList.add("hover"));
        hit.addEventListener("blur", () => group.classList.remove("hover"));
      });
    }

    responsive(chart, draw);

    const headers = [opts.categoryName].concat(opts.series.map((s) => s.name));
    if (multi) headers.push("Total");
    dataTable(root.querySelector(".table-slot"), headers, opts.categories.map((category, i) => {
      const row = [category].concat(opts.series.map((s) => opts.format(opts.value(i, s.key))));
      if (multi) row.push(opts.format(totals[i]));
      return row;
    }));
  }

  // ---------- wire up ----------

  const find = (name) => document.querySelector(`[data-chart="${name}"]`);

  if (find("heatmap") && data.days) heatmap(find("heatmap"));

  if (find("monthly") && data.monthly && data.monthly.length) areaChart(find("monthly"), data.monthly);

  if (find("mix") && data.years && data.years.length) {
    columnChart(find("mix"), {
      categoryName: "Year",
      categories: data.years.map((y) => String(y.year)),
      series: [
        { key: "commits", name: "Commits", slot: 1 },
        { key: "pullRequests", name: "Pull requests", slot: 2 },
        { key: "issues", name: "Issues", slot: 3 },
        { key: "reviews", name: "Reviews", slot: 4 },
        { key: "other", name: "Other", slot: 0 },
      ],
      value: (i, key) => data.years[i][key],
      format: (v) => fmt.format(v),
      unit: "contributions",
    });
  }

  if (find("weekday") && data.weekdays && data.weekdays.length) {
    columnChart(find("weekday"), {
      categoryName: "Day",
      categories: data.weekdays.map((d) => d.name),
      series: [{ key: "average", name: "Average per day", slot: 1 }],
      value: (i, key) => data.weekdays[i][key],
      format: (v) => fmt1.format(v),
      capFormat: (v) => fmt1.format(v),
      unit: "contributions on average",
    });
  }

  if (find("repos") && data.repos && data.repos.length) {
    columnChart(find("repos"), {
      categoryName: "Year",
      categories: data.repos.map((r) => String(r.year)),
      series: [
        { key: "sources", name: "Own", slot: 1 },
        { key: "forks", name: "Forks", slot: 2 },
      ],
      value: (i, key) => data.repos[i][key],
      format: (v) => fmt.format(v),
      unit: "repositories",
    });
  }
})();
