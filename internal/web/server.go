package web

import (
	"bytes"
	"context"
	_ "embed"
	"errors"
	"fmt"
	"html/template"
	"net"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github-stats/internal/github"
)

//go:embed page.html
var pageTemplate string

const maxLanguageSlots = 8

type Server struct {
	listener net.Listener
	server   *http.Server
	page     []byte
}

type pageData struct {
	Stats       *github.UserStats
	Languages   []languageSegment
	CommitBars  []bar
	PRBars      []bar
	ReviewBars  []bar
	GeneratedAt string
}

type languageSegment struct {
	Name  string
	Pct   float64
	Bytes string
	Slot  int
	Style template.CSS
}

type bar struct {
	Label string
	Value string
	Style template.CSS
}

func New(stats *github.UserStats, host string, port int) (*Server, error) {
	page, err := render(stats)
	if err != nil {
		return nil, err
	}

	address := net.JoinHostPort(host, strconv.Itoa(port))
	listener, err := net.Listen("tcp", address)
	if err != nil {
		return nil, fmt.Errorf("failed to listen on %s: %w", address, err)
	}

	s := &Server{listener: listener, page: page}

	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleIndex)
	s.server = &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
	}

	return s, nil
}

func (s *Server) URL() string {
	return "http://" + s.listener.Addr().String()
}

func (s *Server) Run(ctx context.Context) error {
	served := make(chan error, 1)

	go func() {
		err := s.server.Serve(s.listener)
		if errors.Is(err, http.ErrServerClosed) {
			err = nil
		}
		served <- err
	}()

	select {
	case err := <-served:
		return err
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		return s.server.Shutdown(shutdownCtx)
	}
}

func (s *Server) handleIndex(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = w.Write(s.page)
}

func render(stats *github.UserStats) ([]byte, error) {
	funcs := template.FuncMap{
		"count":    humanCount,
		"bytes":    humanBytes,
		"duration": humanDuration,
		"hour":     humanHour,
		"date":     humanDate,
	}

	tmpl, err := template.New("page").Funcs(funcs).Parse(pageTemplate)
	if err != nil {
		return nil, fmt.Errorf("failed to parse page template: %w", err)
	}

	data := pageData{
		Stats:       stats,
		Languages:   languageSegments(stats.Languages),
		CommitBars:  barsFrom(stats.TopReposByCommits),
		GeneratedAt: time.Now().Format("2006-01-02 15:04:05 MST"),
	}
	if stats.PRStats != nil {
		data.PRBars = barsFrom(stats.PRStats.TopRepos)
	}
	if stats.ReviewStats != nil {
		data.ReviewBars = barsFrom(stats.ReviewStats.TopRepos)
	}

	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, data); err != nil {
		return nil, fmt.Errorf("failed to render page: %w", err)
	}

	return buf.Bytes(), nil
}

func languageSegments(languages map[string]int64) []languageSegment {
	if len(languages) == 0 {
		return nil
	}

	langStats := github.GetLanguageStats(languages)

	var segments []languageSegment
	var otherPct float64
	var otherBytes int64

	for i, lang := range langStats.TopLanguages {
		if i >= maxLanguageSlots {
			otherPct += lang.Percentage
			otherBytes += lang.Bytes
			continue
		}
		segments = append(segments, languageSegment{
			Name:  lang.Name,
			Pct:   lang.Percentage,
			Bytes: humanBytes(lang.Bytes),
			Slot:  i + 1,
			Style: widthStyle("flex-basis", lang.Percentage),
		})
	}

	if otherBytes > 0 {
		segments = append(segments, languageSegment{
			Name:  "Other",
			Pct:   otherPct,
			Bytes: humanBytes(otherBytes),
			Slot:  0,
			Style: widthStyle("flex-basis", otherPct),
		})
	}

	return segments
}

func barsFrom(items []github.RepoCount) []bar {
	max := 0
	for _, item := range items {
		if item.Count > max {
			max = item.Count
		}
	}
	if max == 0 {
		return nil
	}

	bars := make([]bar, 0, len(items))
	for _, item := range items {
		bars = append(bars, bar{
			Label: item.RepoName,
			Value: humanCount(item.Count),
			Style: widthStyle("width", float64(item.Count)/float64(max)*100),
		})
	}

	return bars
}

func widthStyle(property string, percentage float64) template.CSS {
	return template.CSS(fmt.Sprintf("%s:%.4f%%", property, percentage))
}

func humanCount(n int) string {
	digits := strconv.Itoa(n)
	sign := ""
	if strings.HasPrefix(digits, "-") {
		sign, digits = "-", digits[1:]
	}

	var out strings.Builder
	for i, digit := range digits {
		if i > 0 && (len(digits)-i)%3 == 0 {
			out.WriteByte(',')
		}
		out.WriteRune(digit)
	}

	return sign + out.String()
}

func humanBytes(b int64) string {
	const unit = 1024
	if b < unit {
		return fmt.Sprintf("%d B", b)
	}
	div, exp := int64(unit), 0
	for n := b / unit; n >= unit; n /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.1f %cB", float64(b)/float64(div), "KMGTPE"[exp])
}

func humanDuration(d time.Duration) string {
	days := int(d.Hours() / 24)
	switch days {
	case 0:
		hours := int(d.Hours())
		if hours == 0 {
			return fmt.Sprintf("%d min", int(d.Minutes()))
		}
		return fmt.Sprintf("%d hours", hours)
	case 1:
		return "1 day"
	default:
		return fmt.Sprintf("%d days", days)
	}
}

func humanHour(hour int) string {
	switch {
	case hour == 0:
		return "12:00 AM"
	case hour < 12:
		return fmt.Sprintf("%d:00 AM", hour)
	case hour == 12:
		return "12:00 PM"
	default:
		return fmt.Sprintf("%d:00 PM", hour-12)
	}
}

func humanDate(t time.Time) string {
	if t.IsZero() {
		return "—"
	}
	return t.Format("Jan 2, 2006")
}
