const UUID_PATTERN =
  /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

export function isTransactionLookupQuery(query: string) {
  const trimmed = query.trim();
  return (
    UUID_PATTERN.test(trimmed) ||
    /^[0-9a-fA-F]{12,64}$/.test(trimmed) ||
    /^tx[:_-]?[a-zA-Z0-9:_-]+$/.test(trimmed)
  );
}
