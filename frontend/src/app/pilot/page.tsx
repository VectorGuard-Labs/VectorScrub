import type { Metadata } from "next";
import Link from "next/link";
import LegalLayout, { Contact, Section } from "@/components/LegalLayout";

export const metadata: Metadata = { title: "Founding customer pilot — VectorScrub" };

/** An https payment link set on the web server (PILOT_PAYMENT_URL), or nothing. */
function paymentUrl(): string | null {
  const v = process.env.PILOT_PAYMENT_URL?.trim();
  return v && v.startsWith("https://") ? v : null;
}

export default function PilotPage() {
  const pay = paymentUrl();
  return (
    <LegalLayout title="Founding customer pilot: $1">
      <Section title="What this is">
        <p>
          A small, one-time test of VectorScrub with a few early customers. It is <strong className="text-white">$1,
          paid once. Nothing renews.</strong> We are testing whether the service is useful, not asking you
          to believe it will make your information disappear. It will not remove you from every site, and
          nothing here guarantees a removal.
        </p>
        <p>
          We are keeping the group small on purpose, so you may find the pilot is full.
        </p>
      </Section>

      <Section title="What you get">
        <ul className="list-disc ml-5 space-y-1">
          <li>A scan of the sites we can currently read, and the listings found for you.</li>
          <li>A plain list of the sites we <strong className="text-white">could not</strong> read, so you know what was not checked.</li>
          <li>Up to three opt-out requests that you authorize one by one (failed attempts do not count against the three).</li>
          <li>Step-by-step help for the opt-outs that must be done by hand, and a way to ask a person for help. You stay in control of every step.</li>
          <li>Status tracking, and follow-up checks on a schedule (about 3, 7, 14 and 30 days after a request, up to 45). A page we cannot read is reported as inconclusive, never as removed.</li>
          <li>A printable report of where each listing stands.</li>
        </ul>
        <p>
          To be straight about it: the scan and three opt-outs are also in the free account. The $1 pays for
          a person&apos;s attention during the pilot and tells us you are serious about trying it.
        </p>
      </Section>

      <Section title="What we ask in return">
        <ul className="list-disc ml-5 space-y-1">
          <li>Honest feedback on what worked and what did not.</li>
          <li>We count anonymized results (for example how many requests were verified). Counts only, never your details.</li>
          <li>Optional: a short follow-up interview. Optional: our asking you if we may share an anonymized version of your results. Each is a separate tick-box when you sign up, and neither is needed to take part. We would ask you again before using anything.</li>
        </ul>
      </Section>

      <Section title="Your information">
        <p>
          We ask for the minimum needed to look you up, encrypt it, and never sell it or use it to train AI
          models. Read the <Link href="/privacy" className="text-teal-400 underline">Privacy Policy</Link> for
          exactly what is stored, who sees it and how to delete it. You can delete your account yourself at any time.
          We will never ask for your email password or ID documents by email.
        </p>
      </Section>

      <Section title="How to join">
        <ol className="list-decimal ml-5 space-y-1">
          <li>
            {pay ? (
              <>
                Pay the one-time $1:{" "}
                <a href={pay} target="_blank" rel="noopener noreferrer" className="text-teal-400 underline">
                  founding pilot payment
                </a>
                . Use the same email address you will register with.
              </>
            ) : (
              <>Ask us for the one-time $1 payment link: <Contact />. Use the same email address you will register with.</>
            )}
          </li>
          <li>
            <Link href="/register" className="text-teal-400 underline">Create your account</Link> and add your details.
          </li>
          <li>We will confirm your place by email.</li>
        </ol>
        <p className="text-gray-400">
          After the pilot we will tell you what a paid plan would include and cost. Nothing is charged
          without you choosing it.
        </p>
      </Section>
    </LegalLayout>
  );
}
