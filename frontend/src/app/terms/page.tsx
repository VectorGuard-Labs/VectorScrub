import type { Metadata } from "next";
import Link from "next/link";
import LegalLayout, { Contact, Section } from "@/components/LegalLayout";

export const metadata: Metadata = { title: "Terms of Service — VectorScrub" };

export default function TermsPage() {
  return (
    <LegalLayout title="Terms of Service">
      <Section title="1. What VectorScrub is, and what it is not">
        <p>
          VectorScrub, a product of VectorGuard Labs (&quot;we&quot;), checks a set of data-broker and
          people-search websites for listings that match you, helps you ask those sites to remove them,
          and re-checks the listings afterwards. By creating an account you agree to these terms and to
          our <Link href="/privacy" className="text-teal-400 underline">Privacy Policy</Link>.
        </p>
        <p>
          <strong className="text-white">We cannot remove you from every site, and we do not promise to.</strong>{" "}
          Many brokers block automated tools, require you to click a link sent to your own inbox, or
          can only be searched by hand. We do not get around those protections. Whether a site removes a
          listing is up to that site.
        </p>
      </Section>

      <Section title="2. This is an early pilot">
        <p>
          The service is new. Parts of it work well, parts do not, and some of it will change. During
          the pilot you should expect that:
        </p>
        <ul className="list-disc ml-5 space-y-1">
          <li>
            We can currently read only some of the sites we list. Your dashboard and the home page show
            which. Finding nothing on a site we could not read does not mean you are not listed there.
          </li>
          <li>
            For sites that block automation, you will be asked to complete the opt-out yourself, using
            the steps we show you. You then tell us you did it, and we check the listing ourselves.
          </li>
          <li>
            Some sites send a confirmation email to you. You must click it, or the request does not count.
          </li>
          <li>
            During the pilot you can ask a person to help with a removal we could not automate. It is
            free, comes with no guaranteed response time, and we will never ask for your email password
            or to get around a site&apos;s own verification.
          </li>
          <li>We may ask for your feedback. We count anonymized results (such as how many requests were verified), never your details. Being interviewed, or being asked about sharing an anonymized version of your results, are separate optional choices at sign-up and are not required.</li>
          <li>Features, limits and prices may change. We will not remove a feature you paid for without telling you first.</li>
        </ul>
      </Section>

      <Section title="3. What you must do">
        <ul className="list-disc ml-5 space-y-1">
          <li>Use VectorScrub only for <strong className="text-white">your own</strong> information. You may not enter another person&apos;s details, or use the service to find or locate someone.</li>
          <li>Give accurate details. A wrong name, address or date of birth leads to wrong matches and failed requests.</li>
          <li>Keep your password private. You are responsible for activity in your account.</li>
          <li>Complete any manual steps and confirmation emails we ask for, if you want those removals to happen.</li>
          <li>Tell us if a listing we report as removed is still there, or one we report as present is not you.</li>
        </ul>
      </Section>

      <Section title="4. Your authorization">
        <p>
          We submit a removal request to a site only after you press the authorize button for that
          specific listing. By pressing it you authorize us to send the opt-out request to that site on
          your behalf, using the details you have given us that the site requires.
        </p>
        <p>
          If a removed listing reappears, we alert you. We do <strong className="text-white">not</strong>{" "}
          submit a new request on our own: you authorize it again.
        </p>
        <p>
          Some sites ask for a copy of an ID or other extra proof. We will not submit that for you
          without asking you first.
        </p>
      </Section>

      <Section title="5. How we decide a listing is &quot;removed&quot;">
        <p>
          We mark a listing removed only when our own re-check finds positive evidence: the page is gone,
          or it loads and no longer shows you. A page we cannot load, a block, a CAPTCHA, or an
          unclear result is reported as <em>inconclusive</em>, never as removed. Telling us you did an
          opt-out by hand is recorded as what you reported; it does not mark anything removed. We
          re-check on a schedule, about 3, 7, 14 and 30 days after you submitted it. Sites vary and some
          take weeks. If a request cannot be
          confirmed within 45 days, it is closed as unverifiable and we tell you.
        </p>
        <p>
          Our checks can be wrong. A listing may exist somewhere we do not look, or may return later.
        </p>
      </Section>

      <Section title="6. Plans and payment">
        <ul className="list-disc ml-5 space-y-1">
          <li><strong className="text-white">Free:</strong> a scan every 7 days across up to 4 sites and up to 3 opt-out requests (requests that fail do not count). Free accounts are not monitored.</li>
          <li>
            <strong className="text-white">Founding pilot:</strong> a one-time payment of $1 for a small group of
            early customers. It does not renew and starts no subscription. It includes what is on the{" "}
            <Link href="/pilot" className="text-teal-400 underline">pilot page</Link>. It guarantees no removal. If the
            pilot is full or ends, we will say so.
          </li>
          <li>
            <strong className="text-white">Paid plans:</strong> we are not yet selling a subscription unless the billing
            page of your account says so. Before you are charged for anything that renews, you will see the
            price and what it includes, and you must choose it. If you buy the Early Access plan ($4.99 per
            month, billed through Stripe), you keep that price while you stay subscribed, even if we raise it for
            new members, and you can cancel at any time from the billing page. Cancelling stops future charges
            and your plan returns to Free at the end of the period you paid for. We do not refund the current
            period, except where the law requires it or we say otherwise in writing.
          </li>
          <li>We do not store your card number. Stripe handles payment.</li>
        </ul>
        <p>
          Because the service cannot reach every site, what you pay buys the work we do and the tracking we
          provide, not a guaranteed outcome. If you think the service did not do what this page says, write to
          us and we will make it right or refund you.
        </p>
      </Section>

      <Section title="7. Ending your account and your data">
        <p>
          You can stop using the service at any time. You can delete your account yourself
          from your profile page, which erases your personal details at once, or write to <Contact /> and we
          will do it for you. Cancel any paid plan first. What we keep afterwards is described in the <Link href="/privacy" className="text-teal-400 underline">Privacy Policy</Link>.
        </p>
        <p>
          We may suspend or close an account that breaks these terms, for example by searching for
          someone else, abusing the service, or attempting to attack it. We will explain why when we can.
        </p>
      </Section>

      <Section title="8. Limits on our responsibility">
        <p>
          The service is provided as it is during the pilot. To the extent the law allows, we are not
          liable for sites that do not honor a request, for listings that return, or for indirect losses.
          Our total liability for any claim is limited to the amount you paid us in the 12 months before
          it. Nothing in these terms limits rights you have that the law does not allow us to limit.
        </p>
        <p>
          VectorScrub is not a law firm and does not give legal advice. It is not affiliated with any
          data broker or with any government service.
        </p>
      </Section>

      <Section title="9. Changes and contact">
        <p>
          If we change these terms in a way that matters, we will tell you before the change applies
          to you, and you may cancel if you do not agree. Questions, problems, and data requests: <Contact />.
        </p>
      </Section>
    </LegalLayout>
  );
}
