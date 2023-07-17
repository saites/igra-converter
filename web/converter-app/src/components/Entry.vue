<script setup lang="ts">
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { ref, computed } from 'vue'
import Accordion from "./Accordion.vue"
import EventsGrid from "./Events.vue"
import EventRow from "./EventRow.vue"
import DataCell from "./DataCell.vue"
import AddressCell from "./AddressCell.vue"
import DbRecordRow from "./DbRecordRow.vue"

const props = defineProps<{
    contestant: Object,
    partners: Array,
    found: Object,
    events: Array,
    issues: Array,
    relevant: Object,
}>()

function fullName(first, last) {
  if (!first) { return last ?? undefined; }
  if (!last) { return first ?? undefined; }
  return `${first} ${last}`
}

const contestantBday = computed(() => {
  let { contestant } = props;
  return `${contestant.dob.month}/${contestant.dob.day}/${contestant.dob.year}`
})

// return true if the given issue's problem matches any of the given fields
function isThisField(issue, fields) {
  return compareTo.some((f) => issue.problem.data.field === f)
}

// fields the registrant didn't fill in
const unfilledFields = computed(() => {
  let { issues } = props;
  if (!issues) { return false; }

  return (issues
    .filter((issue) => issue.problem.name === "NoValue")
    .map((issue) => issue.problem.data.field)
  )
})

// Do we have a database value, but different from this one?
function isDbMismatch(fields) {
  let { issues } = props;
  if (!issues) { return false; }

  return false

  return issues.some((issue) => {
      return (
        issue.problem.name === "DbMismatch"
        && fields.some((f) => issue.problem.data.field === f)
      )
  })
}

const mismatchedFields = computed(() => {
  const { issues } = props;
  if (!issues) { return {} }

  return issues.filter((i) => i.problem.name == "DbMismatch")
    .reduce((acc, f) => {
      acc[f.problem.data.field] = true
      return acc
    }, {})
})

const addressMismatch = computed(() => {
  if (!mismatchedFields?.value) { return false }
  return mismatchedFields.value['AddressLine']
                || mismatchedFields.value['City']
                || mismatchedFields.value['Region']
                || mismatchedFields.value['Country']
                || mismatchedFields.value['PostalCode']
})

// Should we expect _any_ DB data?
const missingDbValue = computed(() => {
  let { issues } = props;
  if (!issues) { return false; }
  return issues.some((issue) => {
    return (
      issue.problem.name === "NotAMember"
      || issue.problem.name === "MaybeAMember"
      || issue.problem.name === "NoPerfectMatch"
    )
  });
})

// Possible matches to the registrant
const maybeMatches = computed(() => {
  let { issues, relevant } = props
  if (!issues || !relevant) { return [] }

  return (
    issues
      .filter((issue) => issue.problem.name === "NoPerfectMatch" && issue.fix.name === "UseThisRecord")
      .map((issue) => relevant[issue.fix.data])
      .filter((record) => record !== undefined)
  )
})

const contestantRegion = computed(() => {
  // The region in the registration form is a full state name,
  // which looks pretty awkward.
  // So, if we have a matching DB value, use that instead,
  // since those are just two letter abbreviations.
  let { issues, contestant, relevant, found } = props
  if (!issues || !contestant || !relevant) { return }
  
  // Make sure we have a DB value to use.
  if (mismatchedFields.value["Region"]) { 
    return contestant.address.region
  }

  return found?.state ?? contestant.address.region
})

const dbContestantCat = computed(() => {
  const { found } = props;
  if (!found) { return }
  return found.sex === "M" ? "Cowboys" : ("F" ? "Cowgirls" : "Unknown")
})

</script>

<template>
  <article>
    <accordion class="py-1 border-dotted border-b-2 border-slate-300">
      <template #summary>
        <div class="px-4 sm:px-6 sm:py-1"
          :class="{'bg-slate-100': issues.length === 0, 'bg-red-200': issues.length > 0}">
        <header class="text-lg">
          <div class="flex flex-row">
          <span class="basis-1/4 md:basis-1/12">{{found?.igra_number ?? "XXXX"}}</span>
          <span class="basis-1/4 md:basis-5/12">
            {{fullName(contestant.firstName, contestant.lastName)}}
          </span>
          <span class="basis-1/4 md:basis-2/12 mx-8">{{events.length}} Go-Rounds</span>
          <span class="basis-1/4 md:basis-4/12">({{issues.length}} 
            issue{{issues.length !== 1 ? 's' : ''}}
            with this registration)
          </span>
          </div>
        </header>
        </div>
      </template>

      <template #content>
        <section>
          <div class="grid grid-cols-3 bg-slate-300">
            <header></header>
            <header class="text-md font-bold">Registration Info</header>
            <header class="text-md font-bold">Database Record</header>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['IGRANumber']}"
              >
              <em>IGRA Number</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['IGRANumber']}"
              :value=contestant.association?.igra></data-cell>
            <data-cell
              :class="{mismatch: mismatchedFields['IGRANumber']}"
                :value=found?.igra_number></data-cell>


            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['Association']}">
              <em>Association</em></span>
            <data-cell 
                  :class="{mismatch: mismatchedFields['Association']}"
                  :value=contestant.association?.memberAssn></data-cell>
            <data-cell 
                  :class="{mismatch: mismatchedFields['Association']}"
                  :value=found?.association></data-cell>

            <span class="fieldHeader"
                :class="{mismatch: mismatchedFields['LegalFirst']}"
                >
              <em>Legal Name</em></span>
            <data-cell
                :class="{mismatch: mismatchedFields['LegalFirst']}"
                :value="fullName(contestant.firstName, contestant.lastName)"></data-cell>
            <data-cell 
                :class="{mismatch: mismatchedFields['LegalFirst']}"
                :value="fullName(found?.legal_first, found?.legal_last)"></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['PerformanceName']}"
              >
              <em>Performance Name</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['PerformanceName']}"
              :value=contestant.performanceName></data-cell>
            <data-cell
              :class="{mismatch: mismatchedFields['PerformanceName']}"
              :value="fullName(found?.first_name, found?.last_name)"></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['DateOfBirth']}"
              >
              <em>Date of Birth</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['DateOfBirth']}"
              :value=contestantBday></data-cell>
            <data-cell 
              :class="{mismatch: mismatchedFields['DateOfBirth']}"
              :value=found?.birthdate></data-cell>

            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['CompetitionCategory']}">
              <em>Competes With</em></span>
            <data-cell
                :class="{mismatch: mismatchedFields['CompetitionCategory']}"
                :value=contestant.gender></data-cell>
            <data-cell 
                :class="{mismatch: mismatchedFields['CompetitionCategory']}"
                :value=dbContestantCat></data-cell>

            <span class="fieldHeader" :class="{ mismatch: addressMismatch }">
              <em>Address</em></span>
              <address-cell
                    :class="{ mismatch: addressMismatch }"
                :line1=contestant.address.addressLine1
                :line2=contestant.address.addressLine2
                :city=contestant.address.city
                :region="contestantRegion"
                :country=contestant.address.country
                :postalCode=contestant.address.zipCode
              ></address-cell>
              <address-cell v-if="found"
                    :class="{ mismatch: addressMismatch }"
                :line1=found.address
                :city=found.city
                :region=found.state
                :postalCode=found.zip
              ></address-cell>
              <data-cell v-else>-</data-cell>

            <span class="fieldHeader"
                 :class="{mismatch: mismatchedFields['Email']}">
              <em>Email</em></span>
            <data-cell 
                 :class="{mismatch: mismatchedFields['Email']}"
                  :value=contestant.address.email></data-cell>
            <data-cell 
                 :class="{mismatch: mismatchedFields['Email']}"
                  :value=found?.email></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['CellPhone']}">
              <em>Cell Phone</em></span>
            <data-cell
              :class="{mismatch: mismatchedFields['CellPhone']}"
              :value=contestant.address.cellPhoneNo></data-cell>
            <data-cell 
              :class="{mismatch: mismatchedFields['CellPhone']}"
              :value=found?.cell_phone></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['HomePhone']}">
              <em>Home Phone</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['HomePhone']}"
              :value=contestant.address.homePhoneNo></data-cell>
            <data-cell
              :class="{mismatch: mismatchedFields['HomePhone']}"
              :value=found?.home_phone></data-cell>

            <span class="fieldHeader">
              <em>Note to Director</em></span>
            <data-cell :value=contestant.noteToDirector></data-cell>
            <span></span>
          </div>

          <accordion v-for="match in maybeMatches">
            <template #summary>
              <div class="ps-10 bg-blue-200 flex flex-row">
                <span class="text-end flex-basis-1/8">Possible Match:</span>
                <span class="px-4 flex-basis-1/8">{{match.igra_number}}</span>
                <span class="px-4 flex-basis-1/4">
                  {{match.legal_first}} {{match.legal_last}}
                </span>
                <span class="ps-4 flex-basis-1/2">
                  {{match.sex === "M" ? "Cowboy" : "Cowgirl"}}
                  from {{match.city}}, {{match.state}}.
                </span>
              </div>
            </template>

            <template #content>
              <div class="grid grid-cols-3 bg-blue-200 pe-24">

                <span class="fieldHeader"><em>Performance Name</em></span>
                <data-cell class="col-span-2"
                  :value="fullName(match.first_name, match.last_name)">
                </data-cell>
                
                <span class="fieldHeader"><em>Date of Birth</em></span>
                <data-cell class="col-span-2" :value=match.birthdate></data-cell>

                <span class="fieldHeader"><em>Address</em></span>
                <address-cell
                    class="col-span-2"
                    :line1=match.address
                    :city=match.city
                    :region=match.state
                    :postalCode=match.zip
                  ></address-cell>
                
                <span class="fieldHeader"><em>Email</em></span>
                <data-cell class="col-span-2" :value=match.email></data-cell>
                
                <span class="fieldHeader"><em>Cell Phone</em></span>
                <data-cell class="col-span-2" :value=match.cell_phone></data-cell>
                
                <span class="fieldHeader"><em>Home Phone</em></span>
                <data-cell class="col-span-2" :value=match.home_phone></data-cell>
              </div>

            </template>
          </accordion>
        </section>
    
        <section>
          <events-grid 
            :events=events
            :issues=issues
            :relevant=relevant
            :partners=partners
          ></events-grid>
        </section>

          <section v-if="issues.length > 0">
            <accordion>
            <template #summary>
              <header class="ps-6 text-lg bg-orange-200">
                {{issues.length}} Problem{{issues.length === 1 ? '' : 's'}}
              </header>
            </template>

            <template #content>
              <div class="grid grid-cols-4">
                  <th class="col-span-2">Problem</th>
                  <th class="col-span-2">Suggestion</th>

                <template v-for="issue in issues">
                  <span>{{issue.problem.name}}</span>
                  <span>
                    <template v-if="issue.problem.data">{{issue.problem.data}}
                    </template>
                  </span>
                  <span>{{issue.fix.name}}</span>
                  <span>
                    <template v-if="issue.fix.data">{{issue.fix.data}}</template>
                  </span>
                </template>
              </div>
            </template>
            </accordion>
          </section>
      </template>
    </accordion>
  </article>
</template>

<style>
.fieldHeader {
  @apply text-end pe-4;
}

.mismatch {
  @apply border-t-2 border-b-2 border-dashed border-red-400 bg-yellow-200;
}
</style>

