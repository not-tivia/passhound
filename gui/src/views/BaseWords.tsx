import { useEffect, useState } from "react";
import BaseWordsHeader from "../components/BaseWordsHeader";
import FavoritesPane from "../components/FavoritesPane";
import AllPane from "../components/AllPane";
import { useToast } from "../components/Toast";
import { useSettings } from "../context/SettingsContext";
import { api } from "../api";
import type { BaseWordView, GuiError } from "../types";

interface BaseWordsProps {
  onLockedError: () => void;
}

export default function BaseWords({ onLockedError }: BaseWordsProps) {
  const toast = useToast();
  const { settings } = useSettings();
  const [words, setWords] = useState<BaseWordView[]>([]);
  const [search, setSearch] = useState("");
  // useState(initial) only uses initial on first render — subsequent changes to
  // settings.default_reveal do not override the user's in-view toggle.
  const [revealAll, setRevealAll] = useState(settings.default_reveal);
  const [analyzing, setAnalyzing] = useState(false);

  const fetchWords = async () => {
    try {
      const result = await api.listBaseWords();
      setWords(result);
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Failed to load base words: ${err.message ?? err.kind}`);
    }
  };

  useEffect(() => {
    fetchWords();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handlePromote = async (id: number) => {
    try {
      await api.promoteBaseWord(id);
      await fetchWords();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Promote failed: ${err.message ?? err.kind}`);
    }
  };

  const handleDemote = async (id: number) => {
    try {
      await api.demoteBaseWord(id);
      await fetchWords();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Demote failed: ${err.message ?? err.kind}`);
    }
  };

  const handleReanalyze = async () => {
    if (analyzing) return;
    if (!window.confirm("Re-analyze password history? This extracts base words from every password (current + retired) and re-ranks the top-10 favorites. May take a few seconds.")) {
      return;
    }
    setAnalyzing(true);
    try {
      const report = await api.analyzeBaseWords();
      await fetchWords();
      if (report.base_words_written === 0 && report.tokens_seen === 0) {
        toast.show("No password history to analyze yet. Import some accounts first.");
      } else {
        toast.show(`Analyzed: ${report.base_words_written} words, ${report.favorites_set} favorites`);
      }
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Analyze failed: ${err.message ?? err.kind}`);
    } finally {
      setAnalyzing(false);
    }
  };

  return (
    <div className="base-words">
      <BaseWordsHeader
        total={words.length}
        revealAll={revealAll}
        analyzing={analyzing}
        onToggleReveal={() => setRevealAll((v) => !v)}
        onReanalyze={handleReanalyze}
      />
      <div className="base-words-panes">
        <FavoritesPane
          words={words}
          revealAll={revealAll}
          onDemote={handleDemote}
        />
        <AllPane
          words={words}
          revealAll={revealAll}
          search={search}
          onSearchChange={setSearch}
          onPromote={handlePromote}
        />
      </div>
    </div>
  );
}
