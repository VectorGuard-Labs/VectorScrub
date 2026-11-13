import type { Metadata } from "next";
import Link from "next/link";
import LegalLayout, { Contact, Section } from "@/components/LegalLayout";

export const metadata: Metadata = { title: "Privacy Policy — VectorScrub" };

export default function PrivacyPage() {
  return (
    <LegalLayout title="Privacy Policy">
      <Section title="1. The short version">
        <ul className="list-disc ml-5 space-y-1">
          <li>We collect only what is needed to find and remove your listings.</li>
          <li>Your identity details are encrypted. We never sell them, never use them to train AI models, and never share them for advertising.</li>
          <li>You can delete your account yourself, and it erases your data at once. Section 7 says exactly what is kept.</li>
          <li>Removing you from a broker means telling that broker who you are. That is unavoidable, and we only do it when you authorize it.</li>
        </ul>
      </Section>

      <Section title="2. What we collect">
        <ul className="list-disc ml-5 space-y-1">
          <li><strong className="text-white">Account:</strong> your email address and a password (stored only as a salted hash).</li>
          <li><strong className="text-white">Identity details you enter:</strong> for example your name, addresses, phone numbers, email addresses and date of birth. These are what we search for and what opt-out forms ask for.</li>
          <li><strong className="text-white">What we find:</strong> the listings that match you (site, link, what kind of information it shows, and a severity score), plus scan history and your privacy score. <strong className="text-white">The link to a listing can contain your name</strong> (some sites put it in the address). Links you add yourself, to listings you found, are handled the same way. We encrypt those links in our database with your own key, like your identity details, and open them only to show them to you or to the scanning software that has to visit them.</li>
          <li><strong className="text-white">Your actions:</strong> removal authorizations, request status, the outcome of each attempt, what you tell us about opt-outs you did by hand (the date, and whether you confirmed the site&apos;s email), whether you asked for help, and a record of when your details were used.</li>
          <li><strong className="text-white">Billing:</strong> your plan, and Stripe customer and subscription identifiers. We do not receive your card number.</li>
          <li><strong className="text-white">Technical:</strong> IP address and browser type in security logs, and a session cookie (see section 8).</li>
        </ul>
      </Section>

      <Section title="3. How we use it">
        <ul className="list-disc ml-5 space-y-1">
          <li>To scan broker sites for listings that match you, and to judge whether a page really is about you.</li>
          <li>To submit opt-out requests you have authorized, and to re-check listings afterwards.</li>
          <li>To alert you, run your account, take payment, prevent abuse, and keep a security record.</li>
          <li>To improve the service using counts and outcomes (for example how many sites we could read, or how many requests were verified), not your identity details.</li>
          <li>If you ticked the optional boxes at sign-up, to contact you about a follow-up interview, or to ask whether we may share an anonymized version of your results. Ticking is not permission to publish: we would ask you first, and you can say no.</li>
        </ul>
        <p>
          We do not sell your information, share it for advertising, or use it to train AI models, ours
          or anyone&apos;s.
        </p>
      </Section>

      <Section title="4. Who else sees it">
        <ul className="list-disc ml-5 space-y-1">
          <li>
            <strong className="text-white">The brokers you authorize us to contact.</strong> To remove a
            listing, the opt-out form or request typically needs your name and often your email or the
            link to your listing. We send only what that site requires, and only for listings you authorized.
          </li>
          <li>
            <strong className="text-white">An AI provider, to help read pages.</strong> To decide whether
            a broker page is about you, we send the page text to a large language model provider. Before
            that, we remove your own details and other people&apos;s names and contact details from the text,
            we do not send your identity or the page link, and we stop the request if any of your details
            remain. This is enforced in our code. The provider is Anthropic.
          </li>
          <li><strong className="text-white">Stripe,</strong> to take payment and manage subscriptions.</li>
          <li><strong className="text-white">Our hosting and infrastructure providers,</strong> who store the encrypted data and run the service for us.</li>
          <li><strong className="text-white">Anyone the law requires us to tell,</strong> if we receive a valid legal demand.</li>
        </ul>
      </Section>

      <Section title="5. How we protect it">
        <ul className="list-disc ml-5 space-y-1">
          <li>Your identity details are encrypted in our database with AES-256-GCM, using a separate key for each customer derived from a master key held only by our backend service.</li>
          <li>Our scanning and removal software does not hold that master key. It receives the minimum details it needs for one task, and each release of your details is recorded.</li>
          <li>The audit log of actions on your account cannot be edited or deleted after it is written.</li>
          <li>Passwords are hashed. Sessions use a cookie that scripts on the page cannot read.</li>
        </ul>
        <p>
          No system is perfectly secure. If we find that your information was exposed, we will tell you
          and say what we know. We do not currently hold a security certification such as SOC 2.
        </p>
      </Section>

      <Section title="6. What we cannot see or control">
        <p>
          Brokers keep their own copies and may share them with others. Removing a listing from one site
          does not remove it from sites we do not cover, and sites can republish it. We also do not read
          your inbox: if a site emails you a confirmation link, you click it.
        </p>
      </Section>

      <Section title="7. How long we keep it, and deleting it">
        <p>
          We keep your account, identity details, scan results, removal requests and alerts for as long
          as your account exists. Privacy-score history older than one year is deleted automatically.
          We do not yet automatically delete inactive accounts.
        </p>
        <p>
          <strong className="text-white">You can delete your account yourself</strong> from your profile
          page (enter your password to confirm). This permanently erases your account, identity
          details, scans, exposures, removal requests, alerts and settings, at once. You can also write
          to <Contact /> and we will do it for you.
        </p>
        <p>What we keep afterwards:</p>
        <ul className="list-disc ml-5 space-y-1">
          <li>
            <strong className="text-white">A security record of what happened.</strong> Our audit logs
            cannot be rewritten, so they stay, but your name, email address, IP address, browser details
            and the free-text notes on removal attempts are removed from them first. What remains says,
            for example, that a removal attempt on some site failed on some date, not whose it was.
          </li>
          <li>
            <strong className="text-white">Payment identifiers.</strong> Stripe holds your payment
            records and may be required to keep them. We keep Stripe&apos;s reference numbers for the
            subscription, cut off from your account, for accounting and disputes.
          </li>
          <li>
            <strong className="text-white">Backups,</strong> until they expire under our hosting
            provider&apos;s schedule. We do not restore deleted accounts from them.
          </li>
        </ul>
        <p>
          Deleting your account does not remove your listings from broker sites. If you have a paid
          plan, cancel it first on the billing page; we cannot delete an account whose subscription would
          keep charging you (one already set to end is fine).
        </p>
      </Section>

      <Section title="8. Cookies">
        <p>
          We use one session cookie to keep you signed in, and a security token that protects form
          submissions. We do not use advertising or analytics cookies.
        </p>
      </Section>

      <Section title="9. Your rights">
        <p>
          Depending on where you live (for example California or the EU/UK) you may have the right to
          access, correct, delete, or receive a copy of your information, and to object to some uses. We
          will honor these for every customer, wherever you live. Write to <Contact />. We will not
          treat you worse for asking.
        </p>
      </Section>

      <Section title="10. Children, changes, contact">
        <p>
          VectorScrub is for adults and is not directed at anyone under 18. If we change this policy in a
          way that matters, we will tell you before the change applies to you. Read it together with the{" "}
          <Link href="/terms" className="text-teal-400 underline">Terms of Service</Link>. Contact: <Contact />.
        </p>
      </Section>
    </LegalLayout>
  );
}
