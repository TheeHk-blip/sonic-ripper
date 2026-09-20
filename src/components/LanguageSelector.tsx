import { useI18n, Language } from '../lib/i18n';

interface LanguageOption {
  code: Language;
  label: string;
}

const LANGUAGES: LanguageOption[] = [
  { code: 'en', label: 'ENG' },
  { code: 'ca', label: 'CAT' },
  { code: 'es', label: 'ESP' },
];

export default function LanguageSelector() {
  const { language, setLanguage } = useI18n();

  return (
    <div
      id="language-selector"
      className="flex items-center rounded-sm bg-charcoal/80 border border-rust/40 p-0.5 text-xs font-mono select-none shadow-sm"
      role="group"
      aria-label="Language selection"
    >
      {LANGUAGES.map(item => {
        const isSelected = item.code === language;
        return (
          <button
            key={item.code}
            id={`btn-lang-${item.code}`}
            type="button"
            onClick={() => setLanguage(item.code)}
            className={`px-2.5 py-1 rounded-[2px] transition-all duration-200 cursor-pointer font-bold tracking-wider ${
              isSelected
                ? 'bg-olive text-cream shadow-xs'
                : 'text-cream/60 hover:text-cream hover:bg-olive/20'
            }`}
          >
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
