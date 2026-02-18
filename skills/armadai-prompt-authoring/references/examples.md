# Prompt Examples

## Example 1: Language Conventions

A prompt that defines coding conventions for a specific language:

```markdown
---
name: java-conventions
description: Boulanger Java coding conventions
apply_to:
  - code-reviewer
  - test-writer
  - java-api-analyzer
---

## Java Conventions

### Naming
- Classes: PascalCase (`OrderService`, `PaymentController`)
- Methods: camelCase (`processOrder`, `validatePayment`)
- Constants: UPPER_SNAKE_CASE (`MAX_RETRY_COUNT`)
- Packages: lowercase, no underscores (`com.company.order.service`)

### Error Handling
- Use custom exceptions extending `BusinessException` for domain errors
- Use `@ControllerAdvice` for centralized exception handling
- Never catch `Exception` or `Throwable` directly
- Always log the root cause before wrapping

### Dependencies
- Use constructor injection (not field injection)
- Annotate required dependencies with `final`
- Use `@RequiredArgsConstructor` from Lombok

### Testing
- Test class naming: `<Class>Test` for unit, `<Class>IT` for integration
- Use AssertJ assertions (not JUnit assertEquals)
- One assertion concept per test method
```

## Example 2: Global Project Context

A prompt providing project context to all agents:

```markdown
---
name: project-context
description: Supply chain API project context
apply_to:
  - "*"
---

## Project: Supply Chain Enriched Stocks API

This API enriches raw stock data from the warehouse system with business metadata.

### Architecture
- **Framework**: Spring Boot 3.x with Platodin overlay
- **Database**: MongoDB (read-heavy, denormalized documents)
- **Events**: Kafka consumer for stock update events
- **Auth**: OAuth2 resource server (company IAM)

### Key Domains
- `Stock` — physical inventory per warehouse/product
- `Enrichment` — business rules applied to raw stock data
- `Notification` — alerts on stock threshold breaches

### Conventions
- All endpoints under `/api/v1/`
- Pagination via `page` and `size` query parameters
- Error responses follow RFC 7807 (Problem Details)
```

## Example 3: Scoped Review Standards

A prompt that only applies to review agents:

```markdown
---
name: review-standards
description: Code review severity definitions
apply_to:
  - code-reviewer
  - security-reviewer
---

## Review Severity Levels

### Critical
Must be fixed before merge:
- Security vulnerabilities
- Data loss risks
- Production-breaking bugs

### Warning
Should be fixed, can be tracked:
- Performance issues
- Missing error handling
- Inconsistent naming

### Info
Suggestions for improvement:
- Code style preferences
- Alternative approaches
- Documentation gaps

Always start with critical findings. If no critical issues, explicitly state "No critical findings."
```

## Example 4: Minimal Prompt (No Frontmatter)

A prompt that relies on filename for its name and has no targeting:

```markdown
# Analysis Output Standards

When producing analysis results:

1. Start with a one-paragraph executive summary
2. Use tables for comparative data
3. Include specific file:line references for code findings
4. End with prioritized action items
```

This prompt must be explicitly listed in `armadai.yaml` to be used — it won't auto-apply to any agent since it has no `apply_to`.
