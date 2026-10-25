/**
 * A small, cached weather lookup for the empty-chat greeting.
 *
 * The approximate location comes from the public IP, not from a setting and not
 * from a device location permission. Both the location and the condition are
 * cached, so opening the welcome screen does not produce a request every time.
 *
 * Nothing here is on a critical path: the clock greeting paints first and a
 * condition replaces it only once the answer is already known.
 */

export type WeatherCondition =
  | "clear"
  | "cloudy"
  | "fog"
  | "drizzle"
  | "rain"
  | "snow"
  | "storm";

type Location = {
  latitude: number;
  longitude: number;
  city: string;
  locatedAt: number;
};

type CachedWeather = {
  condition: WeatherCondition;
  fetchedAt: number;
};

const LOCATION_KEY = "projectz.weather-location.v2";
const WEATHER_KEY = "projectz.weather.v2";

/** The IP's city is stable enough for a day; asking more often buys nothing. */
const LOCATION_FRESH_MS = 24 * 60 * 60 * 1000;
/** A condition older than this is not shown: yesterday's rain is not today's. */
const WEATHER_MAX_AGE_MS = 2 * 60 * 60 * 1000;
/** What counts as current enough to skip a request. */
const WEATHER_FRESH_MS = 30 * 60 * 1000;
/** After a failure, do not hammer either service every time the screen opens. */
const RETRY_AFTER_MS = 5 * 60 * 1000;

let inFlight: Promise<WeatherCondition | null> | null = null;
let lastAttempt = 0;

/**
 * Reads one of the two small stores.
 *
 * Storage is best-effort: a webview with storage disabled still gets weather,
 * just without the cache, and a broken record is treated as no record.
 */
function readStore<T>(key: string): Record<string, T> {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as Record<string, T>) : {};
  } catch {
    return {};
  }
}

function writeStore<T>(key: string, value: Record<string, T>): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // The cache is an optimisation; losing it is not an error worth surfacing.
  }
}

/** One request with a bound, without requiring a newer `AbortSignal.timeout`. */
async function getJson(url: string, timeoutMs = 8_000): Promise<unknown> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok) return null;
    return await response.json();
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

function readLocation(): Location | null {
  const entry = readStore<Location>(LOCATION_KEY).ip;
  if (
    !entry ||
    typeof entry.locatedAt !== "number" ||
    !Number.isFinite(entry.latitude) ||
    !Number.isFinite(entry.longitude)
  ) {
    return null;
  }
  return entry;
}

function locationFrom(payload: unknown): Location | null {
  const value = payload as
    | {
        success?: unknown;
        error?: unknown;
        latitude?: unknown;
        longitude?: unknown;
        city?: unknown;
      }
    | null;
  if (
    value?.success === false ||
    value?.error === true ||
    typeof value?.latitude !== "number" ||
    typeof value?.longitude !== "number"
  ) {
    return null;
  }
  return {
    latitude: value.latitude,
    longitude: value.longitude,
    city: typeof value.city === "string" ? value.city.trim() : "",
    locatedAt: Date.now(),
  };
}

/** The approximate city and coordinates behind this connection's public IP. */
async function locate(): Promise<Location | null> {
  const cached = readLocation();
  if (cached && Date.now() - cached.locatedAt < LOCATION_FRESH_MS) return cached;

  // Two keyless services, one request at a time. The second runs only when the
  // first is unreachable or answers with something unusable.
  let location = locationFrom(await getJson("https://ipwho.is/"));
  if (!location) {
    location = locationFrom(await getJson("https://ipapi.co/json/"));
  }
  if (!location) {
    // A stale location is better than no weather when the lookup itself fails.
    return cached;
  }

  writeStore(LOCATION_KEY, { ip: location });
  return location;
}

function weatherKey(location: Location): string {
  return `${location.latitude.toFixed(2)},${location.longitude.toFixed(2)}`;
}

function readWeather(location: Location): CachedWeather | null {
  const entry = readStore<CachedWeather>(WEATHER_KEY)[weatherKey(location)];
  if (!entry || typeof entry.fetchedAt !== "number" || !entry.condition) return null;
  return Date.now() - entry.fetchedAt <= WEATHER_MAX_AGE_MS ? entry : null;
}

/**
 * What can be painted immediately, or `null` when nothing useful is cached.
 *
 * Synchronous on purpose: it is the hook's initial state, so a recent condition
 * appears in the first frame instead of after two requests.
 */
export function cachedWeather(): WeatherCondition | null {
  const location = readLocation();
  if (!location) return null;
  return readWeather(location)?.condition ?? null;
}

/** The weather code as the small vocabulary the greeting needs. */
export function conditionFor(code: number): WeatherCondition | null {
  if (code === 0) return "clear";
  if (code === 1 || code === 2 || code === 3) return "cloudy";
  if (code === 45 || code === 48) return "fog";
  if ([51, 53, 55, 56, 57].includes(code)) return "drizzle";
  if ([61, 63, 65, 66, 67, 80, 81, 82].includes(code)) return "rain";
  if ([71, 73, 75, 77, 85, 86].includes(code)) return "snow";
  if ([95, 96, 99].includes(code)) return "storm";
  return null;
}

/**
 * Fetches the current condition, reusing both caches.
 *
 * Concurrent calls share a promise, which matters because the welcome screen can
 * be mounted by a conversation switch while the previous lookup is still in the
 * air. A failure backs off for five minutes rather than retrying on every mount.
 */
export function loadWeather(): Promise<WeatherCondition | null> {
  const cachedLocation = readLocation();
  const cached = cachedLocation ? readWeather(cachedLocation) : null;
  if (cached && Date.now() - cached.fetchedAt < WEATHER_FRESH_MS) {
    return Promise.resolve(cached.condition);
  }
  if (Date.now() - lastAttempt < RETRY_AFTER_MS) {
    return Promise.resolve(cached?.condition ?? null);
  }
  if (inFlight) return inFlight;

  const request = (async () => {
    lastAttempt = Date.now();
    const location = await locate();
    if (!location) return cached?.condition ?? null;

    const current = readWeather(location);
    if (current && Date.now() - current.fetchedAt < WEATHER_FRESH_MS) {
      return current.condition;
    }

    const query = new URLSearchParams({
      latitude: location.latitude.toFixed(4),
      longitude: location.longitude.toFixed(4),
      current: "weather_code",
      timezone: "auto",
    });
    const payload = await getJson(`https://api.open-meteo.com/v1/forecast?${query}`);
    const code = (payload as { current?: { weather_code?: unknown } } | null)?.current
      ?.weather_code;
    if (typeof code !== "number") return current?.condition ?? cached?.condition ?? null;

    const condition = conditionFor(code);
    if (!condition) return current?.condition ?? cached?.condition ?? null;

    writeStore(WEATHER_KEY, {
      ...readStore<CachedWeather>(WEATHER_KEY),
      [weatherKey(location)]: { condition, fetchedAt: Date.now() },
    });
    return condition;
  })().finally(() => {
    inFlight = null;
  });

  inFlight = request;
  return request;
}
