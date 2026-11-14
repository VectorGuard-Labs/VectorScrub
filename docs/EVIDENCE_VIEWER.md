# Evidence Viewer System

## Overview
Secure screenshot viewer allowing customers to verify their removal requests were actually submitted.

## Backend API

### Get Evidence Screenshot
```
GET /api/v1/customers/{customer_id}/evidence/{evidence_hash}
Authorization: Bearer {jwt_token}

Response: image/png (raw screenshot bytes)
```

### Get Evidence Metadata
```
GET /api/v1/customers/{customer_id}/evidence/{evidence_hash}/metadata

Response:
{
  "evidence_hash": "abc123...",
  "broker": "spokeo.com",
  "timestamp": 1731499200.0,
  "file_size_bytes": 245678
}
```

## Frontend Component

### React Component
```typescript
// components/EvidenceViewer.tsx
interface EvidenceViewerProps {
  evidenceHash: string;
  broker: string;
  timestamp: number;
}

export function EvidenceViewer({ evidenceHash, broker, timestamp }: EvidenceViewerProps) {
  const [isLoading, setIsLoading] = useState(true);
  const [imageUrl, setImageUrl] = useState<string | null>(null);

  useEffect(() => {
    fetchEvidence(evidenceHash).then(blob => {
      setImageUrl(URL.createObjectURL(blob));
      setIsLoading(false);
    });
  }, [evidenceHash]);

  return (
    <Dialog>
      <DialogTrigger>
        <Button>📸 View Evidence</Button>
      </DialogTrigger>
      <DialogContent className="max-w-4xl">
        <DialogHeader>
          <DialogTitle>Removal Submission Evidence - {broker}</DialogTitle>
          <DialogDescription>
            Screenshot captured: {new Date(timestamp * 1000).toLocaleString()}
          </DialogDescription>
        </DialogHeader>
        {isLoading ? (
          <Spinner />
        ) : (
          <img src={imageUrl} alt="Removal submission evidence" className="w-full" />
        )}
        <DialogFooter>
          <Button onClick={() => downloadEvidence(evidenceHash, broker)}>
            Download Screenshot
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

## Security
- Evidence encrypted at rest using customer-specific keys
- Access control: only owning customer can retrieve
- Audit logged on every retrieval
- Auto-deleted after 90 days (GDPR compliance)

## Storage Requirements
- ~250KB per screenshot (PNG compression)
- 1000 removals/month = 250MB/month
- Keep 90 days = 750MB storage
- Scale with customer growth
