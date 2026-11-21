/// Content languages by their own name, `it` is Italiano everywhere. Flutter has no
/// counterpart of `Intl.DisplayNames`, and a name in its own language reads well for the
/// people who pick it. Tags not listed show as the tag itself.
const _autonyms = <String, String>{
  'ar': 'العربية',
  'cs': 'Čeština',
  'da': 'Dansk',
  'de': 'Deutsch',
  'el': 'Ελληνικά',
  'en': 'English',
  'es': 'Español',
  'fi': 'Suomi',
  'fr': 'Français',
  'he': 'עברית',
  'hi': 'हिन्दी',
  'hu': 'Magyar',
  'it': 'Italiano',
  'ja': '日本語',
  'ko': '한국어',
  'nl': 'Nederlands',
  'no': 'Norsk',
  'pl': 'Polski',
  'pt': 'Português',
  'pt-BR': 'Português (Brasil)',
  'ro': 'Română',
  'ru': 'Русский',
  'sv': 'Svenska',
  'tr': 'Türkçe',
  'uk': 'Українська',
  'zh': '中文',
};

String languageName(String tag) =>
    _autonyms[tag] ?? _autonyms[tag.split('-').first] ?? tag;
